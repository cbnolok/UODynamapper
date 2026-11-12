# Shader settings audit: what each preset setting changes on screen

This note finds the shader settings that are dead, duplicated or too faint to see, and drafts a tooltip for every
setting. It changes no rendering code and no preset value. Every setting of
`dynamapper/assets/defaults/shader_presets.toml` was traced to its Rust struct, its GUI control and the shaders
that read it. Each setting that can act was then rendered in six fixed views, in every style and time of day where
it is reachable. Each was rendered once at the factory value and once neutralised.

The decisions taken on this audit, and the changes made after it, are in [After the audit](#after-the-audit).

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

## After the audit

The audit itself changed nothing. These are the decisions taken on it and what they changed.

### What changed

- **Removed** (never read): `kr_tonemap_params` y, z and w, `gloom_params.w` (Adv "Fog Height Bias"), and the
  alpha of `grade_warm_color` and `grade_cool_color` (now three-value fields).
- **Folded:** `kr_tonemap_params.x` (KR Tonemap Brightness) multiplied exactly like `exposure`, so each KR-profile
  preset's exposure now holds the product, and the field is gone.
  - A cached `land.toml` is migrated on load (`migrate_cached_retired_lighting` in
    `dynamapper/src/configs/shader_presets.rs`): the saved gain moves into the slot's exposure, the fourth values
    are dropped, and the file is rewritten.
  - Pixel identity: the new build against the old one, 16 slots by 4 views, in two launches. Every cell is 0.000
    mean, p99 0, 0% changed, at all four times of day.
- **`blur_radius` kept:** the presets and the slider start at the shader's half-pixel floor
  (`BLUR_RADIUS_MIN_PIXELS`, 0.5), so the value shown is the value used. The identity runs cover this too.
- **Kept as they are:** `sharpening_amount`, `enable_normal_maps` and `land_normal_map_strength`. The Normal Maps
  tooltip says they act only on snow, and only with Enhanced textures.
- **GUI fixes** (numbers from [GUI findings](#gui-findings)):
  1. Fixed: Sun Strength is live in Classic, and Adv Diffuse and Bent normals are shown there.
  2. Fixed: the Color Grading section is shown in every style.
  3. Fixed: the label reads "Rim (Enhanced and KR)"; Rim Color says "KR only", since Enhanced reads only the power.
  4. Gone with `gloom_params.w`.
  5. Fixed: Grade Warm and Grade Cool have controls. Light Color, Rim Color and the grade colours are edited as
     numbers from 0 to 2, because a colour button stops at 1 and these presets go past it. Rim Power has its own
     slider.
  6. Fixed with the floor above.
  7. Gone with the fold.
  8. Fixed: Height Fade runs 0 to 12 on a logarithmic slider.
  9. Fixed: in Classic, Adv Normals no longer writes `normal_mode`.
  10. The composite sliders' tooltips name the settings they drive.
  11. Fixed: the Fog Noise controls are greyed out, with a tooltip, until a fog density reaches 0.001.
  12. As designed; the Water Animation tooltip says the value is kept across presets.
- **Tooltips:** every control of both windows has one, also when greyed out. There are 93 texts in
  `dynamapper/src/core/render/dialogs/land_shader/tips.rs`, one per setting, shared by both windows. They revise
  the drafts below, and cover the client night lighting and Client Shadows controls of KR-like and Enhanced.
- **Shipped `land.toml`:** it now equals the factory presets, and a unit test keeps it so
  (`bundled_land_settings_match_the_factory_presets`). A user's cached copy is still read over it leniently, and
  is never replaced by it.

### Strong values: six kept settings

Six low-impact settings stay. Each was rendered off, at the preset, at twice the preset, at its control's maximum
and beyond it. The beyond values lift the shader clamp in the launch's own shader copy. Each was rendered in
UOD-Painterly and in every other style where it acts once its gate is on, with EC textures at every time of day and
CC textures in the afternoon.

- **Script:** `strong-plan`, `run`, `strong-metrics`, `strong-report`, `strong-sheets` (see [Reproducing](#reproducing)).
- **Sheets:** `/home/claudio/rust-target/uodynamapper-scratch/settings-cleanup/strong/sheets/`, one per setting,
  style and texture set, named `<setting>--<style>--<ec|cc>.png`. Each sheet has a full view and a 2x crop per time
  of day, one column per value, and a change image.
- **Noise:** the same values in two launches differ by 0.000 mean and p99 0.
- **Build:** dev `1d78361a` with the removals above, before KR-like and Enhanced moved to the clients' night
  composite, per-vertex land and (KR-like) client shadows. The KR-like rows were measured with the KR-like lighting
  of that time; UOD-Painterly and Classic are unchanged since.

Numbers are mean / p99 / share over 4 levels, in the view where the change is largest, against the setting off.

| Setting | What it looks like | Preset | Control maximum | Beyond |
|---|---|---|---|---|
| `art_mottle_strength` | Blotchy light and dark patches over roofs and tree canopies, at a few tiles' scale. | 0 (off) | 1.5: 0.54..0.88 / 6..10 / 2.0..6.0% (Painterly and KR-like, Yew); Classic 0.54..0.66 / 4..5 / 0.8..1.9% | 3.0: 1.0..1.7 / 12..19 / 7..13% (Painterly and KR-like); Classic 1.0..1.2 / 9..11 / 5.5..8.5% |
| `kr_land_shadow_mottle_strength` | Breaks the shade on land into soft blotches about two tiles wide. Measured again after the follow-up below: past the presets the blotches deepen and reach sunlit land. | Painterly 0.6: 0.01 / 1 / 0% | 1.5: 3.7..6.0 / 11..17 / 30..64% (Painterly); KR-like per-pixel 5.4..7.2 / 13..21 / 59..66%; CC 6.4..8.2 | none: the slider's end is the strong look (formerly 6.0: 0.11 / 2 / 0.1%) |
| `fill_ground_color` | Light bounced up from the ground onto slopes facing away from the sky. Terrain faces up, so it gets almost no weight. | Painterly: 0.05 / 1 / 0% | Painterly night 0.46 / 2 / 0%; Enhanced with Soft Lighting 0.76 / 6 / 2.7% | 3x: Painterly night 1.02 / 5 / 1.9%; Enhanced 1.99 / 16 / 16% |
| `rim_color` | A thin tint along the edges of slopes turned away from the camera. Measured again after the camera fix and the rim retune below. | Painterly 0.09..0.18 / 2..3 / 0.1..0.5% (before the camera fix 0.09..0.14 / 3..4 / 0.7..1.0%) | hue at 2: 0.16..0.34 / 3..6 / 0.4..1.7% | Rim Power 0.1: 1.9..3.8 / 8..17 / 18..40% |
| `enable_kr_color_wash` | A slow, low-frequency colour drift across land and statics, warmer in some places and cooler in others. | Painterly 0.55..0.74: 0.71..1.14 / 2..3 / 0% | 1.5: 1.50..2.28 / 4..9 / 0.1..10% | 3.0: 2.9..4.4 / 6..16 / 11..34%; KR-like night 4.75 / 10 / 47% |
| `kr_color_wash_scale` | The size of the wash patches, in world units. | 72, wash at preset: at most 1.54 / 3 / 0% between scales | wash at 1.5, scale 8..160: up to 2.69 / 9 / 13.6% | 480: up to 3.75 / 8 / 30.6% (CC 47.5%) |

CC textures give the same picture: art mottle at 1.5 is 0.72..0.82 / 3..6%, the wash at 1.5 is 1.96 / 2.4%, and the
fill ground colour and shadow mottle stay at 0.05..0.07.

Recommendations:

- **`art_mottle_strength`:** keep it off in the presets. It reads as dirt on the art rather than shading; leave it
  as an opt-in for users who want a rougher look.
- **`kr_land_shadow_mottle_strength`:** done: the cap is gone (see the follow-up below).
- **`fill_ground_color`:** keep for tuning. It matters only with a strong Soft Lighting fill, at night or in
  Enhanced; no preset change is needed.
- **`rim_color`:** done: the camera is fixed and the KR rim reaches sunlit slopes (see the follow-up below), so
  the colour shows again at the UOD-Painterly rim, about as faint as before the fix.
- **`enable_kr_color_wash`:** keep. If UOD-Painterly should show the wash, raise its preset towards 1.0..1.5; for
  a stronger range, the slider and the shader clamp would both have to go past 1.5.
- **`kr_color_wash_scale`:** keep, paired with the wash. It shows only when the wash is clearly visible.

### Settings that act with one texture set only

The Render Style picks only shader values, never textures, so every option must work with Classic (CC) and
Enhanced (EC) textures. These act with one set only; their tooltips say so:

- `enable_normal_maps` and `land_normal_map_strength`: EC land only, and only snow has a bump layer.
- Sun Strength and Terrain Ambient Light on water: EC liquid carries its own light and ignores both. CC water uses
  them.
- KR transition grime and KR material contrast skip EC liquid.
- The EC terrain transitions belong to EC land.

All six strong-value settings act with both texture sets.

### Open

- Resolved in the follow-up below: the land camera position, and the Enhanced values its land shading never reads.

### Follow-up: view direction, Enhanced's unread values, shadow mottle

Measured with `strong-plan --settings rim_color,rim_power,specular_strength,kr_land_shadow_mottle_strength` and
`strong-compare`, before (dev `827959e1`) and after, with every factory slot in EC and CC textures. Same-values
noise is 0.000 / 0 in both builds. Sheets: `/home/claudio/rust-target/uodynamapper-scratch/settings-followup/`
(`after/sheets/compare/<style>--<ec|cc>.png` before and after; `after/sheets/<setting>--<style>--<ec|cc>.png`).

- **View direction.** The land shader took its view direction from `SceneUniform.camera_position`, set once when
  the material is built. It now reads Bevy's per-frame view uniform (`world_view_direction` in
  `shaders/world/projection.wesl`): the camera's backward axis for the orthographic camera, the camera position
  for a perspective one. Nothing is written per frame, so the shared material is not re-extracted.
  - Classic, Enhanced and KR-like are pixel identical before and after, at every time of day, in EC and CC.
  - UOD-Painterly changes by 4..20 mean, 44..81% of pixels over 4 levels (EC), and 4..14 mean (CC). All of it is
    its own specular preset (0.02..0.04): the sun (azimuth 315, elevation 60.5) sits behind the scene, so its
    mirror direction is now the camera's for every flat tile (N·H 0.976, 0.976^32 = 0.46), and the specular lays
    an even grey sheen over flat land. Specular at the preset against off: 5.8..19.5 / 19..53 / 53..81%
    (formerly 0.01 / 0 / 0%).
  - The rim lights only shaded slopes facing away from the camera; under this sun there are almost none.
    `rim_color` at the preset: 0.00 / 0 / 0% (formerly 0.09..0.14). Rim Power acts now, faintly: power 0.1 gives
    0.07..0.11 / 2..3 / 0.6..0.9%. Enhanced per-pixel with Rim 0.45: power 0.1 0.08 / 0.6%, power 8 0.03 / 0.2%.
  - UOD-Painterly is retuned below to keep its look from before the fix.
- **UOD-Painterly retune.** Measured against the captures from before the fix (dev `827959e1`), with
  `strong-plan --settings rim_color,rim_power,specular_strength` and the factory slots, EC and CC, every time of day.
  Sheets: `/home/claudio/rust-target/uodynamapper-scratch/painterly-retune/final/sheets/` (`retune/painterly--<ec|cc>.png`:
  before the fix, after it, retuned, with a 2x crop; `<setting>--<style>--<ec|cc>.png` for the three settings).
  - **KR rim** (`kr_rim_grazing` and `kr_rim_sun_weight` in `land/shading.wesl`, mode 2 only). The grazing is now
    measured from flat ground, which the camera sees at a fixed slant: 0 on flat ground and on slopes facing the
    camera, 1 on land seen edge-on. The camera's own grazing (1 - N·V) gave flat ground a rim of 0.12 at power 2.5,
    an even sheen over all land (Rim Colour 5.4 mean, 45% of pixels). The shade-only gate became a weight: full in
    shade, fading to 0.2 at N·L 0.35. Under UOD-Painterly's sun the slopes turned away from the camera face the sun,
    so the rim now shows on sunlit slope edges rather than the shaded ones it lit before the fix. Enhanced (mode 1)
    keeps its rim; Classic and KR-like shade land without one.
  - **Specular.** Any specular now lays the sheen over flat land, so the closest match to the old look is 0. Each
    time of day keeps the largest value under 1% of the pixels over 4 levels against off, in every view, EC and CC:
    morning 0.0015, afternoon 0.001, night 0.0025, cave 0.003 (was 0.035, 0.04, 0.025, 0.02). The next steps up
    (0.002 morning, 0.0015 afternoon, 0.003 night) reach 1.0..2.1% with CC; 0.005 reaches 3.4 / 13 / 30% (EC
    afternoon).
  - At the new presets, against off: specular 0.80..1.11 / 3..4 / 0.0..0.4% (EC), 0.62 / 3 / 0% (CC afternoon);
    Rim Colour 0.09..0.18 / 2..3 / 0.1..0.5% (EC), 0.14 / 3 / 0.3% (CC afternoon). Rim Power: 0.1 gives
    1.9..3.8 / 8..17 / 18..40%, 1 gives 0.48..0.91 / 4..8 / 0.7..4.9%, 8 gives 0.02 / 1 / 0.1%.
  - UOD-Painterly against before the fix, worst view (slopes): 1.0..1.3 / 4..5 / 1.0..1.3% (EC), 0.8..1.2 / 4..5 /
    0.6..1.6% (CC); the fix alone left 7.4..19.5 / 15..53 / 68..81% (EC) and 5.2..14.3 / 21..52 / 44..73% (CC).
    The rest is the rim's new place and the remaining specular.
  - Classic, Enhanced and KR-like are pixel identical to dev `fe633a15` at every time, EC and CC (largest channel
    change 0). Same-values noise is 0.000 / 0.
  - The run used the packages from before the UDDP version 1 reset
    (`/home/claudio/test/uddp-backups/pre-v1-reset-20261112-125210`), which the `fe633a15` rig reads.
- **Shared land rim (2026-11-12).** The rim above moved to `get_rim` in `land/lighting.wesl` (`rim_grazing`,
  `rim_sun_weight`) and serves Enhanced (mode 1) too, which had kept the camera's own grazing and the shade-only
  gate. Enhanced still draws it white at a quarter of the Rim strength (`* 0.30` of the albedo, headroom-capped).
  Measured with `strong-compare` against dev `0ba3c454`, every factory slot, EC and CC; sheets in
  `/home/claudio/rust-target/uodynamapper-scratch/rim-sand/rim-after/sheets/` (`compare/` before and after).
  - Classic, Enhanced, KR-like and UOD-Painterly factory slots are pixel identical at every time of day, EC and
    CC (largest channel change 0): no preset shades Enhanced land per pixel. Same-values noise 0.000 / 0.
  - Enhanced with smooth per-pixel land shading, Rim 0.45, against Rim off (EC afternoon): preset power 2.7
    0.06 / 1 / 0.2% (was 0.05 / 1 / 0.3%); power 0.1 0.30 / 1 / 0.1% (was 0.08 / 2 / 0.6%); power 8
    0.03 / 1 / 0.1% (unchanged). Night and CC are alike. The rim moved from shaded slope edges to the slopes
    turned away from the camera, and stays invisible at any power.
  - The run used `/home/claudio/test/uddp-backups/pre-headers-lights-20261112-153431`: the shared package folder
    was rebuilt in a newer format during the run, which the dev build of that time could not read.
- **Enhanced's unread values.** Enhanced's land shading (mode 1) never reads `diffuse_wrap` (its per-pixel light
  is a plain Lambert), the rim colour (its rim is white) or the headroom reserve (`grade_params.y`; it caps at a
  fixed 0.9). The Enhanced presets now hold their neutral values: 0, white, 0. Every other per-pixel value keeps
  its preset, because the Land Shading option can switch Enhanced to per pixel, where they act once their
  strengths are raised. Enhanced is pixel identical before and after, per vertex, at every time, EC and CC.
  - The Advanced window greys out the per-pixel-only controls in Classic and with per-vertex land shading
    (Specular, Fill, Sharpness, Rim, Rim Power, Fill Sky and Ground, Fill Chroma Tint, Headroom Limit, Gloom),
    and the KR-only ones (Diffuse Wrap, Rim Color, Headroom Reserve, the KR land sliders) unless KR shading is
    per pixel. Their tooltips say why.
- **Shadow mottle.** The darkening was `0.10 * 0.18 * strength`, at most 2.7% at 1.5, in hard-edged tile cells,
  and only on slopes facing away from the sun. Now (`kr_shadow_mottle_depth` in `land/shading.wesl`) the depth
  stays the same up to 0.6, the highest preset, then deepens quadratically to 50% at 1.5; past 0.6 the blotches
  also reach sunlit land, up to 60% of their shaded depth. The pattern is value noise about 2.2 tiles wide.
  - Preset (0.48..0.6): 0.00..0.02 / 1 / 0%, as before.
  - Mid (about 1.05): 0.55..1.26 / 1..4 / 0..0.1%. Slider end 1.5: UOD-Painterly 3.7..6.0 / 11..17 / 30..64% (EC),
    6.4 / 18 / 57% (CC); KR-like per pixel 5.4..7.2 / 13..21 / 59..66% (EC), 8.2 / 26 / 66% (CC).
  - KR-like presets shade land per vertex, which never reads the mottle; its sheets use per-pixel land shading.

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

Every key of `[group.time]`, `[group.time.client_lighting]`, `[group.time.effects]`, `[group.time.lighting]`,
`[group.time.land_lighting]` and `[group.time.sun]` in `assets/defaults/shader_presets.toml` (73 keys). Refreshed
to the presets and controls of 2026-11-12; the audit itself measured the 65 keys and three preset groups of dev
`33242b90`.

- **Rust structs.** The Rust side is one struct per table in
  `src/core/render/scene/world/land/mesh_material.rs`: `LandEffectsUniform`, `GlobalLightingUniforms` and
  `LandLightingUniforms`. `global_lighting` lives in the land scene uniform.
- **Loading.** `src/configs/shader_presets.rs` loads them (`LandMaterialUniformsPresets`).
- **Editing.** The F3 panel (`src/core/render/dialogs/land_shader.rs`) edits a live `UniformState` copy.
- **Columns.**
  - "Main" is the F3 window. "Adv" is the Advanced Raw Shader Knobs window and its collapsing section.
  - Factory values are given as Classic / Enhanced / KR-like / UOD-Painterly (C / E / K / P), with one value when
    the four times of day agree, and otherwise as morning / afternoon / night / cave.
  - "Per pixel" means Enhanced or KR shading with the Smooth per-pixel Land Shading (`terrain_shading_model` 0,
    UOD-Painterly). KR-like and Enhanced shade land per vertex, as their clients do, and never read the per-pixel
    lighting values; the GUI greys those out there.
- **Shaders.** Shader paths are under `dynamapper/assets/shaders/`:
  - land is `world/land/` (`main.wesl`, `shading.wesl`, `lighting.wesl`, `normals.wesl`);
  - art is `world/art/`;
  - fog is `world/effects/fog_pass.wesl`;
  - tonemap and grading are in `postprocess/`.

### Whole scene

| Key | GUI control | Read by | Factory C / E / K / P | Static findings |
|---|---|---|---|---|
| `global_lighting` | Main Brightness; Adv Intensities (Classic, UOD-Painterly) | land, art and ground sprites (final multiply); `static_lights.rs` (night glow alpha) | 1.02/1.08/0.82/0.6 / none / none / 1.04/1.08/0.76/0.56 | It also decides whether the light glow sprites show: their alpha grows with 1 - value. KR-like and Enhanced dim through the client night lighting instead. |
| `client_lighting.light_level`, `overlay_strength`, `overlay_color` | Main Light Level, Night Overlay Strength and Colour (KR-like, Enhanced) | the client night lighting: light buffer and night map | none / 0/0/10/26, 1, the client's colour / same / none | Present only in the client-lit groups, in place of `global_lighting`. |
| `client_lighting.local_light_source` | Main Local Lights (KR-like, Enhanced) | the land light decals and art local lights (`static_lights.rs`): client light sprites or light.mul | none / "light_sprites" / same / none | Matters only with Light/Static Interaction above 0. The light buffer keeps the sprites either way. |
| `effects.shading_mode` | Render Style buttons; Adv Shading Mode | land shading, static light style, gump package | 0 / 1 / 2 / 2 | Selector. The Render Style button replaces the slot value on every switch. |
| `effects.post_process_profile` | Adv Effects Visual Profile | land and art tonemap, KR grime, wash, material contrast, grunge texture, EC art surfaces | 0 / 1 / 2 / 2 | Selector. Like `shading_mode`, the Render Style button replaces it. |
| `effects.texture_filter` | Main Texture Filter | land and sprite sampling | 0 / 0 / 1 / 0 | Pixel art or the KR client's plain bilinear. |
| `effects.enable_linear_filtering` | Main Texture Smoothness; Adv Graphics Linear Filtering | land sampling, EC art item surfaces, ground sprites | 0 / 0 / 1 / 1 | Under Pixel art it softens only the seams between texels. |
| `effects.enable_grunge` | Main Grunge (checkbox); Adv Effects | land, art | 0 / 0 / 0 / 1 | KR also uses it as a land shadow input (`land_surface_normal`). |
| `effects.grunge_strength` | Main Grunge (slider); Adv Effects | land, art | 0 / 0.18 / 0.65/0.65/0.6/0.58 / same | Amount of `enable_grunge`: 0 equals off. |
| `effects.enable_kr_color_wash` | Main KR Color Wash; Adv Graphics | land, art (profile 2 only) | 0 / 0 / 0 / 1 | |
| `effects.kr_color_wash_strength` | Main KR Color Wash (slider); Adv Graphics | land, art (profile 2) | 0 / 0 / 0.55/0.55/0.68/0.74 / same | Amount of the toggle. |
| `effects.kr_color_wash_scale` | Main KR Color Wash Scale; Adv Graphics | land, art (profile 2) | 72 / 72 / 72/72/80/64 / same | Size of the wash pattern; it has no neutral value. |
| `effects.light_decal_intensity` | Main Light/Static Interaction; Adv Graphics | land light pools (the 16 lights nearest the player, every mode except EC liquid, any time of day); art local lights | 1 / 0 / 0 / 1.35 | Art local lights need `world_rendering.enable_static_art_local_lights`, which is off by default. KR-like and Enhanced take the lights from the client light sprites or from light.mul as `client_lighting.local_light_source` picks (Local Lights). |
| `effects.static_light_decal_visibility` | Adv Graphics Static Light Decals (greyed out in KR-like and Enhanced) | Rust only (`static_lights.rs`): alpha of the light glow sprites | 1 / 0 / 0 / 0.55 | Alpha = min(daylight + (1 - global_lighting)^gamma, daylight + max) x value. KR-like and Enhanced light the night through the client light buffer. |
| `lighting.enable_tonemap` | Main Tonemap; Adv Toggles | land, art | 1 everywhere | |
| `lighting.tone_curve` | Main Tone Curve | tonemap (land, art) | 0 / 1 / 1 / 0 | 0 is the visual profile's curve, 1 the identity the clients use (exposure as plain brightness). |
| `lighting.exposure` | Adv Intensities | tonemap (land, art) | 1.05/1.1/1.2/0.95 / 1.05/1.1/1/0.85 / 1.03/1.06/1.06/0.92 / 0.927/0.954/0.954/0.828 | Needs Tonemap on. The retired `kr_tonemap_params.x` (0.9) is folded into the UOD-Painterly values. |
| `lighting.enable_grading` | Main Color Grading; Adv Toggles | land, art, every mode | 0 / 0 / 0 / 1 | The shaders grade in Classic too (`finish_land`). |
| `lighting.grade_params` | Adv Color Grading: x Grade Strength, y Headroom Reserve, z Fill Chroma Tint, w Enable Headroom Limit | x: grading; y, z, w: per-pixel land shading | [0.9, 0.15, 0.35, 1]/[1, 0.15, 0.4, 1]/[1.1, 0.18, 0.45, 1]/[1.05, 0.2, 0.38, 1] / same with y 0 / [1, 0.15, 0.4, 0] / as Classic | x is the amount of `enable_grading`. y is read only in KR per-pixel shading and only with w on; Enhanced uses a fixed 0.10, so its presets hold the neutral 0. z is clamped to 0.20 in Enhanced. |
| `lighting.grade_extra` | Adv Color Grading: vibrance, saturation, contrast, split tone | grading | E as C, P as K: [0.45, 1.2, 1.15, 0.9]/[0.6, 1.25, 1.22, 1]/[0.75, 1.2, 1.3, 0.8]/[0.65, 1.15, 1.28, 0.8] (C, E); [0.38, 1.12, 1.1, 0.75]/[0.45, 1.14, 1.12, 0.8]/[0.22, 1.02, 1.04, 0.65]/[0.22, 1.02, 1.05, 0.65] (K, P) | Identity [0, 1, 1, 0]. Only UOD-Painterly grades by default. |
| `lighting.grade_warm_color`, `grade_cool_color` | Adv Color Grading Grade Warm, Grade Cool | grading split toning | [1.08, 0.98, 0.9], [0.82, 0.95, 1.05] everywhere | They act only through split tone (`grade_extra.w`): warm on bright pixels, cool on dark ones. |
| `lighting.enable_fog` | Main Fog; Adv Effects | fullscreen fog pass | 0 / 0 / 0 / 1 | |
| `lighting.fog_color` | Adv Fog Params (rgb, alpha = max mix) | fog pass | light blue, a 0.5 / same / dark, a 0.3/0.28/0.36/0.34 / same as K | Alpha 0 equals fog off. |
| `lighting.fog_params` | Adv Fog Params: x Distance Density, y Height Density, z Noise Scale, w Noise Strength | fog pass | [0.02, 0, 0, 0] / same / [0.035..0.055, 0.01..0.026, 0.42..0.55, 0.14..0.22] / same as K | The fog is computed in screen space, not world space: x widens a radial vignette and y darkens the lower screen. w is a cloud veil that drifts with time, and z only matters with w. With x and y both under 0.001 the pass returns no fog at all, clouds included. |
| `lighting.fog_night_color` | Adv Night Fog Tint (rgb, blend) | fog pass | 0 / 0 / dark, a 0.25/0.22/0.55/0.6 / same as K | It blends the fog colour, so it needs fog. |

### Terrain

| Key | GUI control | Read by | Factory C / E / K / P | Static findings |
|---|---|---|---|---|
| `land_lighting.terrain_shading_model` | Main Land Shading (Enhanced and KR shading) | land modes 1 and 2 | 0 / 1 / 1 / 0 | 0 smooth per pixel, 1 the clients' half-Lambert grey per cell corner. Per vertex, the per-pixel lighting values below are never read. |
| `effects.normal_mode` | Main Land Relief (composite); Adv Normals | land vertex Lambert (Classic) and fragment normal (per pixel, KR grunge shadow) | 1 / 1 / 0 / 1 | Classic reads it in the vertex shader but shows no change (measured 0.00): at the grid corners where Classic lights vertices, the bicubic normal equals the geometric one. Opening the Advanced window in Classic sets it to 0 (`Geometric (fixed)`). Fragment use stops above zoom 10. |
| `land_lighting.enable_bent` | Main Land Relief (composite); Adv Toggles | land vertex and fragment normal | 1 / 0 / 0 / 1 | It tilts the normal up where a neighbour tile is higher. Acts in Classic and per pixel. Fragment use stops above zoom 20. |
| `effects.enable_blur` | Adv Toggles Blur | land | 0 / 0 / 0 / 1 | Off above zoom 10. |
| `effects.blur_strength` | Adv Intensities | land | 0.15 / 0.15 / 0.28 / 0.28 | Amount of `enable_blur`. |
| `effects.blur_radius` | Adv Intensities (0.5..8 screen pixels) | land | 0.5 everywhere | The shader never blurs less than half a pixel; only 0 turns blur off. |
| `effects.sharpening_amount` | Adv Graphics | land | 0 everywhere | Off in every preset. Not the same as `graphics.toml` `sharpening_strength`. |
| `effects.enable_water_animation` | Main Water Animation; Adv Effects | land, art, ground water | 1 everywhere | Every preset switch keeps the live value, so the slot value counts only for the startup slot. |
| `effects.enable_kr_transition_grime` | Main KR Transition Grime; Adv Graphics | land, profile 2 | 0 / 0 / 0 / 1 | |
| `effects.kr_transition_grime_strength` | Main (slider); Adv Graphics | land, profile 2 | 0 / 0 / 0.65/0.65/0.6/0.58 / same | Amount of the toggle. |
| `effects.enable_normal_maps` | Main Bump Maps; Adv Graphics Land Bump Maps | land, every mode and land source, not liquids | 0 everywhere | EC and KR snow use their shipped bump layer; every other land texture, Classic texmaps included, derives its bumps from its own light and dark texels, fading out as you zoom out. Per pixel the bumps tilt the normal; Classic and the per-vertex model scale their own light by them. Classic land drawn from its land art takes no derived bump. |
| `effects.land_normal_map_strength` | Main (slider); Adv Graphics Land Bump Strength | land, with bump maps | 0.45 everywhere | No preset turns the bumps on. |
| `effects.kr_land_temperature_strength` | Main; Adv Graphics (greyed out unless KR per pixel) | land, KR per pixel | 0 / 0 / 0 / 0.75/0.75/0.7/0.65 | |
| `effects.kr_land_relief_shadow_strength` | Main; Adv Graphics (greyed out unless KR per pixel) | land, KR per pixel | 0 / 0 / 0 / 0.7/0.7/0.62/0.58 | |
| `effects.kr_land_shadow_mottle_strength` | Main; Adv Graphics (greyed out unless KR per pixel) | land, KR per pixel (`kr_shadow_mottle_depth`) | 0 / 0 / 0 / 0.6/0.6/0.52/0.48 | Faint up to 0.6, the highest preset; past it the blotches deepen quadratically to 50% at 1.5 and reach sunlit land too. |
| `effects.kr_land_material_contrast_strength` | Main (greyed out unless KR profile); Adv Graphics | land, profile 2, not liquid | 0 / 0 / 0 / 0.58/0.58/0.48/0.42 | A small saturation and contrast boost by material kind (foliage, stone, bright), with grain per tile. |
| `lighting.light_color` | Adv Lighting Colors | land per pixel (sun colour); art Light Catch | warm by day, cool at night / white / white / as Classic | Classic land and the per-vertex model ignore it. |
| `lighting.atmosphere_tint` | Adv Lighting Colors | art Atmosphere Tint; the gloom tint when `gloom_color` is black | dark blue, the same in every group | UOD-Painterly sets `gloom_color`, so only the art tint reads it. |
| `lighting.ambient_strength` | Main Terrain Ambient Light; Adv Intensities | land in Classic and per pixel; flat water statics; EC item water | 0.18/0.18/0.1/0.06 / 0 / 0 / 0.24/0.25/0.12/0.07 | Per vertex it lights only flat water. |
| `land_lighting.diffuse_strength` | Main Sun Strength; Adv Intensities Diffuse | land in Classic and per pixel; flat water statics; EC item water | 1/1/0.7/0.85 / 1 / 1 / 1.28/1.35/0.62/0.54 | Per vertex it lights only flat water. |
| `land_lighting.specular_strength` | Main Highlights (composite with rim); Adv Intensities | land per pixel (Enhanced x0.5) | 0 / 0 / 0 / 0.0015/0.001/0.0025/0.003 | Under the UOD-Painterly sun, behind the scene, any specular lays an even sheen over flat land; the presets stay under 1% of pixels over 4 levels. The Highlights slider can raise it to 0.25. |
| `land_lighting.rim_strength` | Main Highlights; Adv "Rim (Enhanced and KR)" | land per pixel (Enhanced x0.25, white) | 0 / 0 / 0 / 0.13/0.15/0.16/0.11 | Enhanced and KR share one rim model (`get_rim` in `land/lighting.wesl`): grazing measured from flat ground, raised to the rim power, full in shade and a fifth in sunlight. |
| `land_lighting.fill_strength` | Main Soft Lighting (composite with wrap); Adv Intensities | land per pixel | 0 / 0 / 0 / 0.32/0.34/0.22/0.18 | |
| `land_lighting.sharpness_factor` | Main Land Relief (composite); Adv Intensities | land per pixel, through `sharpness_mix` | 1 / 1.5 / 2 / 2 | No effect when `sharpness_mix` is 0. |
| `land_lighting.sharpness_mix` | Main Land Relief (composite); Adv Intensities | land per pixel (Enhanced clamps it to 0.4) | 0 / 0 / 0 / 0.55 | |
| `land_lighting.diffuse_wrap` | Main Soft Lighting (composite); Adv "Diffuse Wrap (KR only)" | land, KR per pixel | 0 / 0 / 0.5 / 0.35/0.35/0.2/0.15 | KR-like holds 0.5 but shades per vertex, which never reads it. |
| `land_lighting.fill_sky_color`, `fill_ground_color` | Adv Lighting Colors | land per pixel (hemisphere fill, rgb x a) | the same in every group | Both need `fill_strength`. The fill mixes ground into sky by how much a surface faces up, and terrain faces up, so the ground colour is close to unused. |
| `land_lighting.rim_color` | Adv Lighting Colors Rim Color (rgb, KR only) and Rim Power (a) | land per pixel: KR reads rgb and power, Enhanced the power only | [0.7, 0.85, 1, 2.5]/[0.95, 0.95, 0.85, 2.7]/[0.65, 0.85, 1.1, 3]/[0.55, 0.75, 1.05, 2.8] / white, same powers / as C / as C | Enhanced's rim is white, so its presets hold white. |
| `lighting.enable_gloom` | Adv Toggles Gloom (sets `gloom_params.x` to 0.20 when it is 0) | land per pixel | 0 / 0 / 0 / 1 | |
| `lighting.gloom_params` | Adv Gloom: x Amount, y Height Fade Height, z Shadow Bias | land per pixel (`apply_gloom`) | set in every group, used in UOD-Painterly | x is the amount of the toggle. y fades gloom out above that height (0 = no fade). World height is Z x 4/44, so 0.005..0.01 keeps gloom only on land at Z <= 0. The former w ("Fog Height Bias") is gone. |
| `lighting.gloom_color` | Adv Gloom Tint (rgb, desaturation) | land gloom | 0 / 0 / blue-grey, a 0.1..0.28 / same as K | A black rgb falls back to `atmosphere_tint`. |
| `sun.azimuth_degrees`, `sun.elevation_degrees` | Main Sun Azimuth, Sun Elevation | land light direction, art lighting, sprite shadows | 315, 60.5 / 180, 45 / 180, 45 / 315, 60.5 | Under the 315 / 60.5 sun the slopes turned away from the camera face the sun. |

### Art (statics)

| Key | GUI control | Read by | Factory C / E / K / P | Static findings |
|---|---|---|---|---|
| `client_shadows` | Main Client Shadows (KR-like) | art: the KR client's sprite shadows | none / none / on / none | KR-like only; it greys out Shadow Shape, Contact Shadows and Projected Shadows. |
| `effects.art_shadow_strength` | Standard art Shadow Shape; Adv Graphics | art, ground | 0.12 / 0 / 0 / 0.24/0.26/0.3/0.34 | |
| `effects.art_highlight_strength` | Standard art Light Catch; Adv Graphics | art, ground | 0.08 / 0 / 0 / 0.38/0.42/0.28/0.24 | |
| `effects.art_depth_tint_strength` | Standard art Atmosphere Tint; Adv Graphics | art upright sprites (not floors, ground or background) | 0.1 / 0 / 0 / 0.2/0.22/0.3/0.34 | Amount of the `atmosphere_tint` colour. |
| `effects.enable_client_grunge`, `client_grunge_strength` | Standard art Client Grunge; Adv Graphics | art sprites and ground-pipeline statics the KR client grunges (by item id, CC and EC art) | 0 / 0 / 1 / 0, strength 1 | The KR client's `01000045` grunge on statics, mapped as the client does (research P3.6). Never on land or light sprites. |
| `effects.art_contact_shadow_strength` | Standard art Contact Shadows; Adv Graphics | art, only inside the fake-normals block | 0 / 0 / 0 / 0.3/0.34/0.38/0.42 | Needs Fake Normals. |
| `effects.art_mottle_strength` | Standard art Mottle; Adv Graphics | art: shadow-shape noise and fake-normal dapple | 0 everywhere | Off in every preset; visible from about 1.0. |
| `effects.enable_art_fake_normals` | KR-style art Fake Normals; Adv Graphics | art, ground | 0 / 0 / 0 / 1 | |
| `effects.kr_art_temperature_strength` | KR-style art Temperature; Adv Graphics | art, only inside the fake-normals block | 0 / 0 / 0 / 0.75/0.75/0.7/0.65 | Needs Fake Normals. |
| `effects.enable_art_projected_shadows` | KR-style art Projected Shadows (checkbox); Adv Graphics | art | 0 / 0 / 0 / 1 | |
| `effects.art_projected_shadow_strength` | KR-style art Projected Shadows (slider); Adv Graphics | art | 0 / 0 / 0 / 0.22/0.24/0.18/0.16 | Amount of the toggle. |
| `effects.art_projected_shadow_length` | Adv Graphics | art | 0.42 / 0.42 / 0.42/0.44/0.36/0.28 / same | Needs projected shadows. |
| `effects.art_projected_shadow_softness` | Adv Graphics | art (clamped to 0.05..1) | 0.62 / 0.62 / 0.62/0.62/0.68/0.74 / same | Needs projected shadows. |

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

The shipped file now equals the factory presets (see [After the audit](#after-the-audit)). This section describes
the file as it was when the audit ran.

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

The drafts written for this audit are implemented, revised, in
`dynamapper/src/core/render/dialogs/land_shader/tips.rs`; the option buttons keep theirs next to their choices in
`land_shader.rs` (`TONE_CURVE_OPTIONS`, `TERRAIN_SHADING_OPTIONS`, `TEXTURE_FILTER_OPTIONS`). The tables below are
those texts as of 2026-11-12, with the UOD-Painterly specular of 0.001..0.003, the rim Enhanced and KR share, the
Enhanced neutral values, the land bump maps for every land source and the deeper shadow mottle.

There is one tooltip per setting. Wherever a setting has two controls (the main panel and the Advanced window), the
same text serves both. The amounts share their switch's text unless they have their own. Settings without a control
have no tooltip.

### Main panel: window buttons

| Control | Setting | Tooltip |
|---|---|---|
| Time of day buttons | the preset slot | Loads this render style's preset for the time of day. Your edits to the current preset are kept until you save or undo. |
| Show Advanced | none | Opens every raw shader value in a second window. Its controls edit the same values as the ones here. |
| Reset Current Preset to Factory Defaults | the current slot | Puts the current render style and time of day back to the factory values. |
| Undo to Last Saved | every value | Restores the values saved when this window was last closed. |

### Main panel: whole scene

| Control | Setting | Tooltip |
|---|---|---|
| Tonemap | `enable_tonemap` | Compresses bright light so sunlit areas keep their detail instead of washing out. On in every preset; turning it off brightens the frame, most in Classic and Enhanced. |
| Tone Curve | `tone_curve` | Which tone curve Tonemap uses. Needs Tonemap on. |
| Tone Curve: Profile | `tone_curve` 0 | The visual profile's curve (Reinhard, EC or KR filmic), exposure inside it. |
| Tone Curve: Identity | `tone_curve` 1 | No curve, as the clients draw: exposure is plain brightness. |
| Color Grading | `enable_grading` | Turns on the colour grading tuned under Advanced > Color Grading, in every render style; on in UOD-Painterly. If colours look oversaturated, lower Saturation or Vibrance there. |
| Texture Smoothness | `enable_linear_filtering` | Softens the seams between texture pixels under the Pixel art filter; liquids and item surfaces follow it under either filter. On in KR-like and UOD-Painterly. |
| Texture Filter | `texture_filter` | How land and item textures are sampled: crisp pixel art, or the KR client's plain bilinear (KR-like). |
| Texture Filter: Pixel art | `texture_filter` 0 | Nearest texels; Texture Smoothness softens only their seams. |
| Texture Filter: Client bilinear | `texture_filter` 1 | As the KR client: plain bilinear on land and sprites. Mip-like averaging only when zoomed out past its 18.4-tile view. |
| Fog | `enable_fog` | Adds haze toward the screen edges and the bottom, with slowly drifting cloud patches. On in UOD-Painterly; shape it under Advanced > Fog Params. |
| Brightness | `global_lighting` | Overall light level of the scene in Classic and UOD-Painterly, whose presets lower it at night and in caves; KR-like and Enhanced dim through Light Level instead. The darker the scene, the more light sources glow. |
| Light Level | `client_lighting.light_level` | The server's light level: 0 is full day (no change), about 10 a night, 26 a dungeon. KR-like and Enhanced, in place of Brightness. |
| Night Overlay Strength | `client_lighting.overlay_strength` | The night map's tint. 1 is the client; 0 darkens as much without the blue; above 1 tints more. Needs a Light Level above 0. |
| Night Overlay Colour | `client_lighting.overlay_color` | The night map's mean colour. The client's keeps its blue; another colour keeps its cloud pattern. |
| Night Overlay Colour: Client | `client_lighting.overlay_color` | Back to the client's colour, the night map's exact mean. |
| Light/Static Interaction | `light_decal_intensity` | How brightly lamps, torches and other light sources light the ground around them, at any time of day. Visible in Classic and UOD-Painterly, off in Enhanced and KR-like. |
| Local Lights | `client_lighting.local_light_source` | Which lights light the ground through Light/Static Interaction in KR-like and Enhanced, and the statics too when world_rendering.enable_static_art_local_lights is on. Light Sprites: the client light sprites, each lighting the ground under its glow, as large as the sprite and in its texture's colour. light.mul: the Classic lights, as in Classic and UOD-Painterly: every static with the light source flag, with its light.mul light's size and its hue or light colour. Either way the night still glows with the client light sprites. Needs Light/Static Interaction above 0 (0 in these presets). |
| Local Lights: Light Sprites | `client_lighting.local_light_source` "light_sprites" | The client light sprites light the ground under their glow, as large as the sprite and in its texture's colour. |
| Local Lights: light.mul | `client_lighting.local_light_source` "light_mul" | The Classic lights from light.mul, as in Classic: every static with the light source flag, with its light's size and its hue or light colour. |
| Grunge | `enable_grunge`, `grunge_strength` | Lays a dirt and wear texture over land and items. Strong in UOD-Painterly at every time; the Enhanced amount is too light to see. |
| KR Color Wash | `enable_kr_color_wash`, `kr_color_wash_strength` | Adds large, soft patches of colour variation over land and items, like a painted wash. Needs the KR visual profile (KR-like, UOD-Painterly); at the UOD-Painterly amount it is very faint, from about 1.5 it shows as broad blotches. |
| KR Color Wash Scale | `kr_color_wash_scale` | Size of the colour wash patches; larger values make bigger patches. Needs KR Color Wash on and matters only at a strong wash. |

### Main panel: terrain and water

| Control | Setting | Tooltip |
|---|---|---|
| Water Animation | `enable_water_animation` | Moves the water surfaces of land and statics. Water only; it keeps its value when you switch presets and stops when zoomed far in. |
| Land Shading | `terrain_shading_model` | How Enhanced and KR styles light land: smoothly per pixel (UOD-Painterly), or the clients' per-corner grey (KR-like, Enhanced). Classic always uses its own per-corner light. |
| Land Shading: Smooth per-pixel | `terrain_shading_model` 0 | Lighting per pixel from a smooth normal, with every land lighting control. |
| Land Shading: Client per-vertex | `terrain_shading_model` 1 | As the KR and Enhanced clients: a half-Lambert grey per cell corner, blended over two triangles. |
| Sun Strength | `diffuse_strength` | How strongly the sun lights land that faces it: the main light and shade on terrain in Classic and UOD-Painterly. With Client per-vertex land shading (KR-like, Enhanced) it lights only flat water; water drawn from Enhanced textures carries its own light and ignores it. |
| Sun Azimuth | `sun.azimuth_degrees` | Compass bearing of the sun in degrees, clockwise from north (180 = south). Lights land and art and turns the sprite shadows. |
| Sun Elevation | `sun.elevation_degrees` | Height of the sun above the horizon in degrees (90 = overhead). |
| Terrain Ambient Light | `ambient_strength` | Light that reaches every land tile, even in shade, in Classic and with per-pixel land shading; raise it to open up dark slopes, nights and caves. With Client per-vertex land shading it lights only flat water; water drawn from Enhanced textures ignores it. |
| Land Relief | `normal_mode`, `enable_bent`, `sharpness_mix`, `sharpness_factor` | Brings out the shape of hills with smoother normals and a crisper edge between sun and shade. Per-pixel land shading only (UOD-Painterly); visible on slopes, not on flat ground. |
| Soft Lighting | `fill_strength`, `diffuse_wrap` | Adds soft light from the sky and lets sunlight reach further around slopes, lifting the shade. Per-pixel land shading only; strongest at night in UOD-Painterly. |
| Highlights | `specular_strength`, `rim_strength` | Adds a shine where land reflects the sun and a thin light on the edges of slopes turned away from the camera. Per-pixel land shading only; both are faint at the UOD-Painterly values. |
| KR Land Temperature | `kr_land_temperature_strength` | Warms sunlit land and cools shaded land. KR shading with per-pixel land shading (UOD-Painterly); visible by day. |
| KR Land Relief Shadow | `kr_land_relief_shadow_strength` | Darkens land at the foot of height steps and along cliff banks. KR shading with per-pixel land shading (UOD-Painterly); visible by day. |
| KR Land Shadow Mottle | `kr_land_shadow_mottle_strength` | Breaks the shade on land into soft blotches. KR shading with per-pixel land shading (UOD-Painterly). Faint on shaded slopes up to about 0.6, the UOD-Painterly value; past it the blotches darken and spread over sunlit land too, for a gloomy, cloud-shaded look at the maximum. |
| KR Material Contrast | `kr_land_material_contrast_strength` | Makes grass, rock and other land textures slightly richer and more varied from tile to tile. Needs the KR visual profile (UOD-Painterly), not on water; strongest by day. |
| KR Transition Grime | `enable_kr_transition_grime`, `kr_transition_grime_strength` | Dirties and darkens the edges where one kind of land meets another. Needs the KR visual profile (UOD-Painterly); clear by day, faint at night. |
| Bump Maps | `enable_normal_maps`, `land_normal_map_strength` | Gives land textures small bumps that catch the sun. Enhanced and KR snow use the bump map the client ships with it; every other land texture, Classic texmaps included, derives its bumps from its own light and dark texels. Works in every render style and land source: per-pixel land shading lights the bumps directly, Classic shading and the clients' per-vertex shading brighten and darken their own light by them. Water and other liquids stay flat. The derived bumps fade out as you zoom out. Off in every preset. |

### Main panel: standard art and KR-style art

| Control | Setting | Tooltip |
|---|---|---|
| Shadow Shape | `art_shadow_strength` | Darkens the shaded side of items and buildings to give them volume. Clear in UOD-Painterly, too light to see in Classic; greyed out in KR-like, where Client Shadows replace it. |
| Contact Shadows | `art_contact_shadow_strength` | Darkens items where they meet the ground. Needs Fake Normals on (UOD-Painterly); greyed out in KR-like, where Client Shadows replace it. |
| Mottle | `art_mottle_strength` | Adds blotchy light and dark patches over items such as roofs and tree canopies, in every render style. Off in every preset; visible from about 1.0. |
| Light Catch | `art_highlight_strength` | Brightens the sunlit side of items and buildings. Clear in UOD-Painterly; too light to see in Classic. |
| Atmosphere Tint | `art_depth_tint_strength` | Fades upright items toward the Atmosphere Tint colour (Advanced > Lighting Colors) for a sense of depth. Clear in UOD-Painterly; too light to see in Classic. |
| Client Grunge | `enable_client_grunge`, `client_grunge_strength` | The KR client's grunge: soft blotches of its grunge texture on the statics it marks, every 16 tiles. On in KR-like. |
| Client Shadows | `client_shadows` | The KR client's shadows of statics and items, leaning from their base (its Show Shadows option). KR-like only, where they replace Shadow Shape, Contact Shadows and Projected Shadows. |
| Temperature | `kr_art_temperature_strength` | Warms the sunlit side of items and cools their shaded side. Needs Fake Normals on (UOD-Painterly); faint, mostly by day. |
| Projected Shadows | `enable_art_projected_shadows`, `art_projected_shadow_strength` | Casts a soft shadow from each item onto the ground. On in UOD-Painterly; greyed out in KR-like, where Client Shadows replace it. |
| Fake Normals | `enable_art_fake_normals` | Shades items as if they had depth, guessing their shape from the sprite. On in UOD-Painterly; Contact Shadows and Temperature need it. |

### Advanced window

| Control | Setting | Tooltip |
|---|---|---|
| Effects: Visual Profile | `post_process_profile` | Which look's tone curve and style-only effects are used: Neutral, EC or KR. The Render Style buttons set it; the KR effects need KR here. |
| Effects: Grunge Strength | `grunge_strength` | How strong the dirt and wear texture is. Needs Grunge on. |
| Effects: Shading Mode | `shading_mode` | How land is lit: per corner (Classic) or per pixel (Enhanced, KR). It decides which terrain settings act; the Render Style buttons set it. |
| Effects: Normals | `normal_mode` | Bicubic uses the slope of the neighbouring tiles for rounder hills; Geometric shades each tile's own slope. Visible on slopes with per-pixel land shading; Classic always uses Geometric. |
| Toggles: Bent normals | `enable_bent` | Evens out the light on land below higher ground, such as the foot of a slope or a dip. Acts in Classic and with per-pixel land shading; on in Classic and UOD-Painterly. |
| Toggles: Gloom | `enable_gloom`, `gloom_params.x` | Darkens and cools shaded land at sea level for a moodier look. Enhanced and KR shading with per-pixel land shading (on in UOD-Painterly); clearest at night. It and the Gloom section are greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Toggles: Blur | `enable_blur` | Softens land textures slightly, in every render style, until zoomed far in. On in UOD-Painterly; land only. |
| Intensities: Exposure (Tonemap) | `exposure` | Brightens or darkens the whole frame before the tone curve. Needs Tonemap on; the presets raise it a little by day. |
| Intensities: Blur Strength | `blur_strength` | How much of the blurred texture is mixed in. Needs Blur on. |
| Intensities: Blur Radius | `blur_radius` | How far the land blur reaches, in screen pixels; the shader never blurs less than half a pixel. Needs Blur on. |
| Intensities: Specular | `specular_strength` | A shine where land reflects the sun. With the UOD-Painterly sun, behind the scene, it lays an even grey sheen over all flat land: faint at the UOD-Painterly values, clear by day from about 0.002. Per-pixel land shading only; it is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Intensities: Fill (env) | `fill_strength` | Soft light from the sky added to all land, lifting the shade. Per-pixel land shading only; strongest at night in UOD-Painterly. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Intensities: Sharpness Factor | `sharpness_factor` | How crisp the line between sunlit and shaded land gets. Acts only through Sharpness Mix; greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Intensities: Sharpness Mix | `sharpness_mix` | How much of the crisper sun and shade line is used. Visible in UOD-Painterly at night and in caves; greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Intensities: Rim (Enhanced and KR) | `rim_strength` | A thin light on the edges of slopes turned away from the camera, at full strength in shade and a fifth of it in sunlight. Enhanced and KR per-pixel shading share it; Enhanced draws it white at a quarter of the strength, too faint to see even at the slider maximum. Faint at the UOD-Painterly values. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Intensities: Diffuse Wrap (KR only) | `diffuse_wrap` | Lets sunlight reach further around slopes, softening the edge of the shade. KR shading only, greyed out unless KR shading has per-pixel land shading, since Enhanced, Classic and the per-vertex model never read it; Enhanced's per-pixel light is a plain Lambert. |
| Lighting Colors: Light Color | `light_color` | Colour of the sunlight on land and on the lit side of items; values above 1 brighten. On land only with per-pixel land shading (UOD-Painterly, strongest at night); Classic land and the clients' per-corner land ignore it. |
| Lighting Colors: Atmosphere Tint | `atmosphere_tint` | Colour that upright items fade toward with Atmosphere Tint, and the gloom colour when Gloom Tint is black. Visible in UOD-Painterly. |
| Lighting Colors: Fill Sky | `fill_sky_color` | Colour and strength (alpha) of the sky light from Soft Lighting. Per-pixel land shading only; its colour is hard to see. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Lighting Colors: Fill Ground | `fill_ground_color` | Colour and strength (alpha) of the soft light from below, which reaches only land facing down, such as cliff overhangs. Per-pixel land shading with Soft Lighting; barely visible even at full strength. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Lighting Colors: Rim Color (KR only) | `rim_color` rgb | Colour of the rim light on land; values above 1 brighten it. KR shading only, greyed out unless KR shading has per-pixel land shading, since Enhanced, Classic and the per-vertex model never read it; Enhanced's rim is white. Faint at the UOD-Painterly rim: a tint along the edges of slopes turned away from the camera. |
| Lighting Colors: Rim Power | `rim_color.a` | How thin the rim light is: higher keeps it to land seen closer to edge-on, so less of each slope catches it. In UOD-Painterly it is clear from about 1 down to 0.1, the widest rim; Enhanced's rim stays too faint to see at any power. Per-pixel land shading with some Rim; greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Color Grading: Grade Strength | `grade_params.x` | How much of the colour grading is applied; 0 is the same as Color Grading off. |
| Color Grading: Headroom Reserve | `grade_params.y` | How far below white the KR light on land is capped, so sunlit ground does not bleach. KR shading with per-pixel land shading, by day, with Enable Headroom Limit on; greyed out unless KR shading has per-pixel land shading, since Enhanced, Classic and the per-vertex model never read it; Enhanced caps at a fixed 0.9. |
| Color Grading: Fill Chroma Tint | `grade_params.z` | How much the sky light's colour tints land in shade. Enhanced and KR per-pixel shading; very faint. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Color Grading: Enable Headroom Limit | `grade_params.w` | Caps the light on land so sunlit ground keeps its colour instead of washing out. Enhanced and KR shading with per-pixel land shading, by day; turning it off brightens daylit land a lot. It is greyed out in Classic and with Client per-vertex land shading (KR-like, Enhanced), which never read it. |
| Color Grading: Vibrance | `grade_extra.x` | Boosts dull colours more than vivid ones. Needs Color Grading on. |
| Color Grading: Saturation | `grade_extra.y` | Strength of all colours; 1 leaves them unchanged. Needs Color Grading on. |
| Color Grading: Contrast | `grade_extra.z` | Pushes darks and lights apart; 1 leaves them unchanged. Needs Color Grading on. |
| Color Grading: Split Tone Strength | `grade_extra.w` | Tints bright areas with Grade Warm and dark areas with Grade Cool; 0 is off. Needs Color Grading on. |
| Color Grading: Grade Warm | `grade_warm_color` | Tint of bright areas in the split tone; only its hue matters, not its brightness. Needs Color Grading and Split Tone Strength; hard to see at the preset. |
| Color Grading: Grade Cool | `grade_cool_color` | Tint of dark areas in the split tone; only its hue matters, not its brightness. Needs Color Grading and Split Tone Strength; clearly visible in UOD-Painterly. |
| Gloom: Amount | `gloom_params.x` | How strong the gloom is. Needs Gloom on; switching Gloom on sets it to 0.2 when it is 0. |
| Gloom: Height Fade Height | `gloom_params.y` | Height above which the gloom fades out, in world units (land spans about -12 to +12). The presets keep it to land at sea level; 0 covers land at every height. |
| Gloom: Shadow Bias | `gloom_params.z` | Puts more of the gloom on land facing away from the sun. Visible in UOD-Painterly at night and in caves. |
| Gloom: Gloom Tint | `gloom_color` | Colour of the gloom, and in alpha how much colour it drains. Black uses the Atmosphere Tint; visible in UOD-Painterly at night. |
| Graphics: Sharpening Amount | `sharpening_amount` | Sharpens land textures, in every render style. 0 in every preset; even 0.5 changes most of the frame. |
| Graphics: KR Color Wash Strength | `kr_color_wash_strength` | How strong the colour wash is. Needs KR Color Wash on and the KR visual profile. |
| Graphics: Land Bump Strength | `land_normal_map_strength` | How strong the land bumps are: how far a bump map tilts the surface, or how deep the bumps derived from a texture are. Needs Land Bump Maps on. |
| Graphics: Static Light Decals | `static_light_decal_visibility` | Strength of the glow drawn around light sources. Shows at night and in caves in Classic and UOD-Painterly, and grows as Brightness drops; greyed out in KR-like and Enhanced, which light the night through the client's light buffer. |
| Graphics: KR Transition Grime Strength | `kr_transition_grime_strength` | How strong the grime on land borders is. Needs KR Transition Grime on and the KR visual profile. |
| Graphics: Art Projected Shadow Strength | `art_projected_shadow_strength` | How dark the item shadows are. Needs Projected Shadows on. |
| Graphics: Art Projected Shadow Length | `art_projected_shadow_length` | How long the item shadows are. Needs Projected Shadows on. |
| Graphics: Art Projected Shadow Softness | `art_projected_shadow_softness` | How soft the edge of the item shadows is. Needs Projected Shadows on; the difference is small. |
| Fog Params: Fog Color | `fog_color` | Colour of the haze, and in alpha how thick it can get (0 turns fog off). Needs Fog on. |
| Fog Params: Distance Density | `fog_params.x` | How far the haze reaches in from the screen edges. Needs Fog on; with this and Height Density at 0 there is no fog at all. |
| Fog Params: Height Density | `fog_params.y` | Extra haze toward the bottom of the screen. Needs Fog on; too faint to see at the UOD-Painterly values. |
| Fog Params: Noise Scale | `fog_params.z` | Size of the drifting cloud patches. Needs Fog on, some Distance or Height Density, and Noise Strength above 0. |
| Fog Params: Noise Strength | `fog_params.w` | How visible the slowly drifting cloud patches are. Needs Fog on and some Distance or Height Density: the clouds only shape existing haze. |
| Fog Params: Night Fog Tint | `fog_night_color` | A colour mixed into the haze, by the amount in alpha; the presets use more at night and in caves. Hard to see at the preset values. |

The other Advanced controls repeat a main-panel control and take its text: Fog, Grunge / Weathering, Water
Animation, Tonemap, Color Grading, Global Lighting (Brightness), Ambient, Diffuse (Sun Strength), and the Graphics
entries for linear filtering, colour wash, light decals, land bump maps, the KR land effects and the art
strengths.

Two drafts belong to controls that are gone, and are kept as a record:

| Control | Setting | Draft |
|---|---|---|
| KR Tonemap Brightness | `kr_tonemap_params.x` | Brightens or darkens the KR image before its tone curve. KR style only; it does the same as Exposure, and the two multiply. (Folded into Exposure.) |
| Gloom: Fog Height Bias | `gloom_params.w` | Not used by the renderer. (Removed with the fourth gloom component.) |

## Where the tooltips go

This section describes the code as it was when the audit ran. Every helper now takes the tooltip text, and the
line numbers have moved.

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
- **Strong values.** Point `SHADER_AUDIT_SCRATCH` at a folder of its own, put a rig binary built with the
  F3 slider helpers set to `SliderClamping::Edits` there (a rig-only change, so beyond values are not clamped), and
  run `strong-plan`, `run`, `strong-metrics`, `strong-report` and `strong-sheets`. The beyond launches lift two
  shader clamps in their own shader copy (`CLAMP_LIFTS`); `strong-plan --settings a,b` keeps only some settings.
- **Before and after a build.** Run the same `strong-plan` in two scratch folders, each with its own rig binary
  and a pinned copy of `dynamapper/assets` in `SCRATCH/assets`, then `strong-compare BEFORE_FOLDER` in the later
  one: `results/compare.md` holds every capture's change, and `sheets/compare/` one before, after and change sheet
  per render style and texture set.

## Related notes

- [effect_visibility_validation.md](effect_visibility_validation.md): normal-map inputs (only snow has a bump
  layer), the bundled `land.toml` overriding the factory presets, and earlier effect measurements.
- [static_light_visibility.md](static_light_visibility.md): how the light pools and glow sprites are gated.
