# Render Style Rules

The hard rules each render style follows: Classic, Enhanced, KR-like and UOD-Painterly. Check here first. Each rule is
one or two sentences with its source, the code that implements it and the test or fixture that guards it; the linked
documents hold the details and are not repeated here.

Where the styles differ, a table shows them side by side; a rule stated once holds in every style.

**Abbreviations**

| Short | Meaning |
|---|---|
| `art/` | `dynamapper/src/render/statics/` |
| `world/` | `dynamapper/src/render/scene/world/` |
| `sh/` | `dynamapper/assets/shaders/world/` |
| `conv/` | `lib/udd-conv/src/textures/` |
| CC client | Classic `client.exe` 7.0.103.0 (PE FileVersion) |
| EC client | Enhanced `UOSA.exe` 4.0.102.3 |
| KR client | Kingdom Reborn `UOKR.exe` 2.58.0.6 (its folder's `buildinfo.txt` says 2.53.0.2) |
| CUO | ClassicUO's source, a read-only Classic reference |
| Fixture | a view in [`fixtures.toml`](../../../skills/renderer-reference-validation/fixtures.toml), run by the regression harness |
| Test | a unit test, named by its function |

Client addresses and their traces are in [`docs/client_research/`](../../client_research/README.md).

## 1. Styles and sources

### What a style is

A style is a shading mode plus a preset group. There are three shading modes and four preset groups: KR-like and
UOD-Painterly share shading mode 2 and the KR post-process profile and differ only by their presets.

| | Classic | Enhanced | KR-like | UOD-Painterly |
|---|---|---|---|---|
| Preset group | `classic` | `enhanced` | `kr` | `painterly` |
| Shading mode, post profile | 0 (per vertex), 0 | 1 (per fragment), 1 | 2 (per fragment), 2 | 2 (per fragment), 2 |
| Aim | the Classic client | the Enhanced client | the KR client, client terms only | KR content, every effect |
| Scene multiplier `global_lighting` | own | none (client) | none (client) | own |
| Night dimming | Classic rule, no night map | client composite + night map | client composite + night map | client composite, `overlay_strength` 0 |
| Lights | light.mul decals | client light sprites | client light sprites | client light sprites + glow |
| "clients" light cover | EC client's | EC client's | KR client's | KR client's |
| Player's floor rule | CUO | CUO | choice, default KR | choice, default KR |
| Static shadows (bundled) | none | none (the EC client draws none) | KR client shadows | projected + contact sprite shadows |
| Sun | baked | client sun | client sun | baked |
| Land texture variation | none | on | on | on |
| Client texture filter, grunge, chroma knee | off | off | on | off |
| Fog (bundled) | off | off | off (the KR client draws none) | on |

- **The chroma knee softens the land and the foliage alone.** KR-like's knee (0.55, slope 0), tuned to bring the KR
  textures' grass to the KR screenshots, acts on the land, on the statics that continue it (`static_continues_land`:
  grass, liquids, swamp, lava fields) and on the foliage (Foliage in the tile data the static is drawn by,
  `drawn_tile_data`); furniture, walls, trunks and the other statics keep their own colour. Source: user, 2026-11-18
  (the knee on every material turned the shops' counters and the Minoc tables the grey-brown of their floors; without
  it the leaves stood out of the KR screenshots' foliage). Code: `STATIC_FLAG_LAND_GRADE`, `land_grade_flag`
  (`art/statics_collect.rs`), `grade_color_vibrant`'s `land_grade` (`postprocess/color_grading.wesl`). Test:
  `canonical_statics_that_continue_the_land_take_its_style`. Fixtures: `britain-shop-counter-keeps-its-colour`, `minoc-hall-tables-zview`, `minoc-grass-land-like-statics`,
  `yew-forest-canopy`.
- **Projected shadows follow the sun and the casters' shapes.** UOD-Painterly's sprite shadows lie on the plane of
  the caster's base and lean away from the sun: a point h world units up falls h × Length × (the sun's run over the
  default sun's, at most 4×) along the ground, so azimuth turns them and elevation lengthens or shortens them; an
  overhead sun casts none. Only what the KR client casts a shadow for casts one (no Roof, Surface, Rotated or
  NoShadow item, no floor, Background static or EC item surface), so a roof's or a stair's tiles no longer shade the
  tiles beside them. A wall (the light scene's wall class) casts its two faces, the south and the east edge of its
  cell, as a parallelogram along the ground, with its doors and windows read through its art; trees and the other
  casters keep the card laid on the ground in a soft ellipse. A shadow is as dark as the sun's share of flat
  ground's light, Sun Strength × the sun's height ÷ (ambient + that), times its Strength: a stronger or higher sun
  darkens the shadows, more ambient light lightens them, as a real sun does. UOD-Painterly's Strengths are that
  share's inverse at their own sun, so the presets' shadows are as dark as before. Source: user, 2026-11-25 (roof
  tiles shaded one another, walls cast round blobs, the sun settings did not move the shadows; Sun Strength had no
  visible effect). Code: `projected_shadow_casts`, `projected_shadow_step`, `projected_shadow_sun_share`,
  `wall_shadow_face_point`, `wall_shadow_card_corner` (`sh/art/projected_shadow.wesl`), the `PASS_MODE_SHADOW`
  branches of `sh/art/main.wesl`. Tests: `projected_shadow_tests.rs`,
  `painterly_shadows_and_headroom_look_as_before_at_their_own_sun`. Known gap: a shadow lies at its caster's base z,
  so an upper storey's wall casts a band in the air over the storey under it.
- **Sun Strength lights the land in every style.** It is Classic's and UOD-Painterly's sun term, and in KR-like and
  Enhanced it scales the clients' per-vertex grey, which is all sun light: 1, their presets', is the clients' light.
  The per-pixel headroom ceiling, 1 − Headroom Reserve (Enhanced: 0.9), rises in proportion to a Sun Strength above
  1, so the cap no longer swallows the slider (UOD-Painterly's day sun lit flat land to 1.75× the old fixed ceiling,
  and the slider did nothing from about 0.65 up); UOD-Painterly's day reserves are re-expressed so that the ceiling
  at its own sun stays 0.85. Source: user, 2026-11-25 (Sun Strength did not look effective). Code:
  `land_headroom_ceiling` (`sh/land/shading.wesl`), the client per-vertex branch of `shade_land` (`sh/land/main.wesl`).
  Tests: `painterly_shadows_and_headroom_look_as_before_at_their_own_sun`, `gpu_the_headroom_ceiling_rises_with_the_sun`,
  `a_new_sun_strength_reaches_the_art_materials`.
- **The art shading controls reach every style that shows them.** Without Fake Normals (Classic, Enhanced), Shadow
  Shape and Light Catch keep their strength as their amount up to a knee at 0.15, above the Classic presets' 0.12 and
  0.08, and rise past it to 1.85 and 2 at the slider's end: a wall's foot about two thirds darker, its top about twice
  as bright. With the strength itself the end of the slider darkened a foot by a third and brightened a top by half,
  gradients the tiling of wall pieces hid. Contact Shadows, which did nothing without Fake Normals, darkens an upright
  sprite's lower part toward its foot, from 40 % of its height and in full from 85 % (where a wall meets the ground
  along the tile's lower edges), by up to 70 % at 1; each wall piece darkens at its own foot, so upper storeys band at
  high values. Flat art, which has no foot, takes none. Mottle's patches are three times their former amplitude,
  about ±⅛ on a wall and ±⅓ on a roof at 1.5. Every preset keeps its look: Contact Shadows is 0 wherever Fake
  Normals are off, Mottle is 0 everywhere, and the fake-normal path (UOD-Painterly) is unchanged. Source: user, 2026-11-25 (Shadow Shape, Contact Shadows, Mottle and Light Catch showed
  no effect on Classic art). Code: `apply_art_surface_shading`, `art_plain_amount` (`sh/art/shading.wesl`). Tests:
  `gpu_the_art_controls_change_classic_art_plainly`, `gpu_shadow_shape_and_light_catch_keep_the_classic_presets`,
  `gpu_mottle_breaks_up_the_art_plainly` (`dynamapper/src/render/shading_reach_tests.rs`),
  `classic_art_strengths_lie_below_the_plain_knee`.
- **The KR Color Wash shows past its knee.** Up to 0.75, above every preset's strength, the wash's amount is the
  strength, a shift of a few percent at most, so the presets look as before; past it the amount rises to 10 at 1.5,
  where the patches shift the value by up to 0.4 and take the warm or the cool wash's colour in full. The
  patches are Scale tiles wide; the presets' 64 to 80 are wider than the screen, so the wash changes only as you
  travel, and the slider now goes down to 4. Source: user, 2026-11-25 (the wash did not look effective). Code:
  `kr_color_wash_amount` (`dynamapper/assets/shaders/postprocess/kr_color_wash.wesl`). Tests:
  `gpu_the_kr_color_wash_keeps_the_presets_and_shows_past_the_knee`, `the_kr_color_wash_sliders_match_the_shader`.
- **The fog drifts over the map, never from the screen's edges.** Fog is soft cloud banks, a domain-warped gradient
  noise read on the camera plane at the map position under each pixel, so the banks pan with the map, and the wind
  carries them Drift Speed tiles a second toward its bearing while their warp moves at 0.55 of it, so they change
  shape as they go. Each pixel mixes toward the fog colour by up to Amount where a bank is thickest; nothing depends
  on the pixel's place on the screen, so the corners fog no more than the centre. The night tint turns the colour a
  dark blue at night and in caves, and the client composite then dims it with the scene. On in UOD-Painterly alone:
  the KR client draws no fog (its sky and fog code is dead), and KR-like's moving cloud look at night is the night
  map's world-anchored blotches. Source: user, 2026-11-26 (the fog whitened the four corners; it should be a slight
  cloudy moving layer). Code: `fog_uniform` (`dynamapper/src/render/postprocess/fog_pass.rs`), `fog_density`
  (`sh/effects/fog_noise.wesl`). Tests: `a_map_point_keeps_its_cloud_when_the_camera_pans`,
  `the_wind_carries_the_fog_toward_its_bearing`, `gpu_coverage_runs_from_wisps_to_a_veil`,
  `gpu_the_fog_repeats_on_its_period_and_has_no_grain`, `only_uod_painterly_has_the_fog_on_and_every_slot_shares_its_clouds`.

- Source: user decisions recorded in [`CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`](CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md);
  KR client trace [`uokr_atmosphere.md`](../../client_research/ghidra-kr-atmosphere/results/uokr_atmosphere.md).
- Code: `RENDER_STYLE_PRESETS` (`dynamapper/src/ui/dialogs/shading.rs`); the `preset_group_*` predicates
  in `dynamapper/src/settings/shader_presets.rs` (`preset_group_has_scene_multiplier`, `preset_group_has_night_map`,
  `preset_group_draws_light_sprites`, `preset_group_follows_kr_light_rules`, `preset_group_uses_client_shadows`,
  `preset_group_has_player_floor_rule`, `preset_group_has_land_texture_variation`); `validate_preset_lighting`.
- Guard: tests `painterly_presets_are_their_own_slots`, `bundled_presets_dim_every_style_by_the_light_level`,
  `the_night_overlay_controls_are_every_style_s_but_classic_s`, `bundled_presets_give_the_light_sprite_styles_their_local_lights`,
  `only_kr_like_has_the_client_shadows_and_draws_them_alone`, `player_floor_rule_is_kr_likes_and_painterlys_alone`,
  `bundled_presets_keep_the_baked_sun_but_kr_like_and_enhanced_take_the_client_sun`,
  `bundled_presets_turn_the_land_texture_variation_on_in_every_style_but_classic`,
  `only_kr_like_takes_the_client_texture_filter`, `only_kr_like_has_the_client_grunge_on`, `only_kr_like_has_the_chroma_knee_on`.

### Sources are not part of the style

The art source (CC, EC) and the land source (CC, EC, KR) are preferences of their own; a preset never picks them, so
every combination is valid in every style. The rules of sections 2, 5 and 7 that name a source follow the source, not
the style.

| | Values | Bundled default | Harness default per style |
|---|---|---|---|
| Art (`graphics.art_texture_source`) | `cc` (`tex_art_cc.uddp`), `ec` (`tex_art_ec.uddp`) | `ec` | Classic cc/cc and ec/ec; Enhanced ec/ec; KR-like ec/kr and ec/ec; Painterly ec/ec |
| Land (`graphics.land_texture_source`) | `cc`, `ec`, `kr` (the KR section of `tex_land_ec.uddp`) | `ec` | (pairs are art/land) |

- **EC art is KR art.** The EC client draws an item's world art (tileart block 0) only with `UseNewArt`, otherwise its
  Classic image; our EC art source draws block 0 for every item that has a slot, a project choice. Source: EC client
  `UOTileArt::BuildSpriteDescriptor` 0x004592e0 ([world-art windows §3](../../client_research/world-art-windows/README.md));
  user, 2026-10-31. Code: `resolve_static_visual_kind_cached` (`art/statics_collect.rs`); an item without an EC slot
  falls back to its Classic art (`present_classic_static_visual_kind`). Test: `cc_visual_kind_prefers_owner_slot_with_texture_fallback`,
  `canonical_art_inventory_uses_kr_then_classic_then_missing`.
- **Statics that continue the land take the land's style**, whatever the art source: under CC land EC art draws them
  from their Classic art, under EC or KR land CC art draws them the Enhanced way. Built floors (NoShoot) keep their
  art source. Source: user decisions (f6b7b590, dfbd8422). Code: `static_continues_land`, `StaticLandStyle::of_land_atlas`.
  Tests: `canonical_statics_that_continue_the_land_take_its_style`, `statics_continue_the_land_when_enhanced_draws_them_from_the_terrain_and_they_are_not_built_floors`,
  `classic_art_keeps_its_art_without_enhanced_land`. Fixtures: `minoc-grass-land-like-statics`, `britain-shop-floors`, `acid-pools`, `swamp-statics`.
- **KR items only under KR land.** Under the KR land, a static that continues the land is drawn from the KR item
  sections (art, material, tile data) where they hold it; nowhere else does any style draw the KR art section.
  Code: `kingdom_reborn_static_visual_kind`, `KrStaticPackages::sources` (filtered on `StaticLandStyle::KingdomReborn`).
  Tests: `kingdom_reborn_kinds_read_the_kingdom_reborn_records`, `canonical_statics_that_continue_the_kingdom_reborn_land_take_its_items`.
  Fixtures: `deceit-kr-rotated-surfaces`, `minoc-mine-mouth-kr-land-night`.

## 2. Geometry

| Rule | Source | Code | Guard |
|---|---|---|---|
| A cell is the 44 × 44 px Classic diamond; one z unit is 4 px, `UO_Z_TO_WORLD_SCALE` = 4/44 world units. | CC art grid | `dynamapper/src/util_lib/uo_coords.rs`, `sh/uo_coords.wesl`; `CC_TILE_PIXEL_WIDTH` (`art/statics_collect.rs`) | Test `upright_art_projects_square_pixels_and_four_pixels_per_z_unit` |
| World axes: x = UO x, z = UO y, y up. The isometric camera is orthographic at `PlayerCamera::BASE_OFFSET_FROM_PLAYER` (5, 5, 5), looking along (-1, -1, -1); the lens stretches view y to make the diamond. | CC view | `dynamapper/src/render/view/camera.rs` | Tests `both_cameras_look_along_the_isometric_diagonal`, `default_camera_matches_legacy_tile_pixel_and_height_steps` |
| The perspective camera (`graphics.perspective_camera`) orbits along the same `Vec3::ONE` direction at the distance that matches the orthographic framing, with the same lens stretch; no sprite-only scale. | Dynamapper ([isometric perspective notes](../../agent_notes/isometric_perspective_projection.md)) | `PlayerCamera::perspective_offset` | Tests `both_cameras_look_along_the_isometric_diagonal`, `orbit_distance_gives_the_focus_the_orthographic_scale_at_any_window_and_zoom`; fixtures `*-perspective` (e.g. `britain-castle-wall-walk-perspective-wall-tops`) |
| The land quad of a cell splits along its NW–SE diagonal (corner 0 to 3), in every style and LOD; the cave mouth veil and the picking mesh cut theirs the same way. | Dynamapper | `QUAD_TRIANGLES` (`terrain/setup_base_mesh.rs`), `build_cell_quads_mesh` | Test `mouth_quads_lie_in_their_chunk_on_its_mesh_grid` |
| Sprite anchors: CC art stands at (x + 1, y + 1), EC art at (x + 0.5, y + 1.5), the KR client's sprite base point. Both add 2 to the row. | KR client `UOSprite::PlaceSprite` 0x004422b0; CC symmetric centring | `CC_STATIC_TILE_TRANSLATION_X/Z`, `EC_STATIC_TILE_TRANSLATION_X/Z` | Tests `enhanced_sprites_stand_where_the_clients_place_world_art`, `regular_static_art_keeps_sprite_world_anchor` |
| EC art is cropped to its record's enhanced world-art window (ends + 1), offset by its draw offset and scaled by `world_art_scale`: the clients' mode 1. | EC/KR client descriptor builders ([world-art windows §3](../../client_research/world-art-windows/README.md)) | `selected_art_window`, `build_sprite_geometry` (`conv/tex_art_ec.rs`) | Tests `enhanced_billboard_offsets_use_enhanced_pixel_ratio`, `enhanced_billboard_bounds_normalize_to_classic_world_footprint` |
| The 215 EC records whose window is a copy of the legacy window (no `UseNewArt`, same image and scale as KR) take KR's window and draw offset when packed. | KR client reads its own window; user, 2026-11-17 (70e54459) | `adopt_kr_world_windows`, `load_ec_sources_with_kr_windows` (`conv/kr_items.rs`) | Tests `an_enhanced_record_takes_the_kingdom_reborn_window_of_the_same_image_and_scale`, `the_enhanced_records_take_the_215_kingdom_reborn_windows` (udd-conv integration); fixture `ter-mur-statuettes-kr-windows` |
| An item that borrows another item's image through a copied window no client reads gets no EC slot when the window cuts more than `COPIED_WINDOW_MAX_CUT_SHARE` (2 %) of its alpha; it draws its Classic art (ship 0xA432/0xA440, orchids 0xA645–0xA648). | 70e54459 | `has_unread_copied_window`, `copied_window_cut_share` (`conv/tex_art_ec.rs`) | Tests `a_borrowed_image_through_a_copied_window_is_unread_unless_the_record_uses_new_art`, `the_cut_share_is_the_alpha_outside_the_cut_window` |
| Upright statics are camera-facing cards (`flat_quad` 0). | CC/KR clients | `STATIC_BILLBOARD_RIGHT_XZ` | Test `upright_art_projects_square_pixels_and_four_pixels_per_z_unit` |
| CC flat art (`flat_quad` 1) lies flat: the Wet statics, and a dry 44 × 44 Background + Surface diamond of height 0 (not Wet, Roof, Foliage, Bridge) whose four land corners are at its z. | CC tiledata flags | `cc_static_tile_renders_flat`, `cc_ground_diamond`, `cc_static_uses_flat_geometry` | Tests `cc_flat_render_gate_targets_wet_water_not_surface_floors`, `cc_ground_diamond_requires_tile_sized_dry_background_without_volume`, `dry_cc_picking_and_rendering_require_all_four_resident_level_corners` |
| EC item surfaces (`flat_quad` 2) lie flat over their cell; a Rotated item is an authored EC camera quad (`flat_quad` 3), a card at the EC anchor. | EC/KR tileart Rotated | `EC_CAMERA_QUAD_FLAG`, `ec_static_camera_quad`, `FLAT_QUAD_EC_SURFACE`, `FLAT_QUAD_EC_CAMERA` | Test `ec_camera_quad_uses_the_authored_flag_instead_of_wet_classification`; fixtures `deceit-swamp-rotated`, `deceit-swamp-rotated-perspective` |

## 3. Draw order

One rule in every style and both cameras: ClassicUO's (decision 2026-11-16, accepted by the user 2026-11-16). Details,
removed rules and benchmarks: [`DEPTH_SCALE_AND_KR_CLIENT.md`](DEPTH_SCALE_AND_KR_CLIENT.md).

| Rule | Source | Code | Guard |
|---|---|---|---|
| The nearer diagonal row x + y always wins, whatever the heights; within a row the priority z decides, then the tie level. | CUO `Map.cs AddGameObject`, `GameScene` draw | `logical_draw_distance`, `static_depth_key` (`art/statics_collect.rs`); `sh/art/depth.wesl` | Tests `a_nearer_row_beats_any_priority_z`, `within_a_row_priority_z_then_tie_level_decide`, `gpu_depth_follows_the_cpu_draw_order_in_both_cameras` |
| `L = row + (priority z + tie level / 9) / 264`. 264 exceeds the priority z range, so z never crosses a row. | 2026-11-16 decision | `PRIORITY_Z_STEPS_PER_ROW`, `PRIORITY_TIE_LEVELS_PER_Z` (`depth.wesl` and `statics_collect.rs`) | Test `gpu_depth_follows_the_cpu_draw_order_in_both_cameras` |
| Static priority z = tile z, − 1 for Background, + 1 when the tiledata height is not 0, + 1 for MultiMovable. Surface adds nothing. | CUO | `resolve_priority_z_units` | Tests `priority_z_is_classicuo_z_with_its_adjustments`, `surface_like_tables_keep_classicuo_priority`, `bridge_stone_and_stairs_retain_volume_depth_and_height_priority` |
| Land priority z = its average z − 2. The average z is CUO's `Land.AverageZ`: the mean of the corner pair along the diagonal that rises less. | CUO | `LAND_PRIORITY_Z_OFFSET`, `land_cell_average_z` (`sh/land/main.wesl`); `average_land_z` (`world/z_view.rs`) | Test `average_land_z_takes_the_flatter_diagonal`; fixtures `hill-tree-line-rising-ground`, `slope-fallen-logs` |
| Tie levels: 0 for land, flat quads (1, 2) and ground art; upright sprites 1 + 2 × min(rank, 3) + (x mod 2), the later static of a cell and priority z nearer. A KR client shadow takes its caster's level − 1. | CUO (later wins) | `assign_sprite_tie_levels`, `SPRITE_TIE_CELL_RANKS` | Tests `later_statics_of_one_cell_and_priority_z_draw_nearer`, `equal_statics_of_neighbouring_cells_differ_by_x_parity_across_chunks`, `flat_quads_keep_tie_level_zero` |
| One depth per cell: the land, the flat quads, the ground art and the projected shadows take their cell's anchor, corner + (1, 1), for every fragment. EC camera quads take the EC anchor. | CUO draws a cell's objects whole in its row | `CELL_DEPTH_ANCHOR`, `cell_priority_view_z` (`depth.wesl`); `cell_depth_anchor`, `sprite_depth_anchor` | Tests `a_cells_surfaces_share_its_anchor_row_ahead_of_its_land`, `ec_camera_quads_take_the_ec_art_anchor_row`; fixtures `britain-upper-floors-flat-roofs`, `minoc-hall-tables-zview` |
| The land writes its depth per fragment from a flat varying of its cell and average z (`depth_cell`); fragments of another cell (coarse LODs, banks) read the corners from the tile atlas. | 2026-11-16 (4616c17c) | `sh/land/main.wesl` | Test `gpu_depth_follows_the_cpu_draw_order_in_both_cameras` |
| Under MSAA the land's world position is interpolated at the centroid of the samples a fragment covers, so an edge fragment takes its own cell's colour, height and depth, not the next cell's. | Dynamapper (MSAA) | `LandVertexOutput.world_position` (`sh/land/main.wesl`) | Fixture `minoc-river-water-msaa` |
| Perspective wall caps: a wall with a Surface or Bridge laid on its top in the cell west or north moves its cap fragments to the farther row, at the top's z − 2 and the highest tie level. Walls under a roof or another wall, and every wall in the orthographic camera, keep their caps. | Dynamapper (82b0f0fc) | `wall_cap_meets_floor_flag`, `STATIC_FLAG_CAP_MEETS_FLOOR`; `perspective_cap_may_move`, `perspective_cap_row_shift`, `perspective_cap_priority_z` (`depth.wesl`) | Test `only_a_wall_with_a_floor_on_its_top_in_the_farther_row_moves_its_cap`; fixtures `britain-castle-wall-walk-perspective-wall-tops`, `britain-house-gable-palisade-perspective` |
| Projected shadows take the depth of the cell they fall on, at the caster's base z − 0.5, level 0. KR client shadows sort at their caster's priority depth. | KR client 0x00435120 (bias), P3.4 | `PROJECTED_SHADOW_PRIORITY_Z_OFFSET`, `client_shadow_logical_view_z` (`sh/art/main.wesl`); `CLIENT_SHADOW_TIE_LEVEL_OFFSET` (`sh/art/client_shadow.wesl`) | Test `shadows_sort_between_their_ground_and_their_caster`; fixture `britain-castle-wall-walk-kr-shadows` |
| Caves drawn over the map are lifted toward the camera in view z. | Dynamapper | `cave_lifted_view_z` (`depth.wesl`) | Fixture `minoc-mine-entrance-zview-cave` |
| A mobile's frame below its anchor is ordered nearer, over the land of the rows it lies over: a Classic frame in one slice cut at its anchor, two rows nearer (an ophidian's tail, body 86); an Enhanced frame in four strips cut 8 px over its anchor, two rows a strip. Lower still, the land of the next row covers it, and a static of the strip's rows can draw behind it, as in the clients. Same priority z and tie level as the card; shadows keep the card's depth. | CUO `MobileView.DrawInternal`; UOSA `MobAnimTriShape::RenderImmediate` 0x004328b0 | `MobileGroundStrips::of` (`mobiles/draw.rs`), `mobile_ground_strip` (`depth.wesl`) | Tests `a_classic_tail_lies_over_the_flat_land_in_front`, `a_classic_frame_comes_nearer_below_its_anchor_in_one_slice`, `the_enhanced_legs_lie_over_the_flat_land_in_front`, `only_the_enhanced_frames_below_the_cut_come_nearer`. No fixture: the harness cannot show the player body |
| The pick pass and the light scene use the same depth; the light scene accepts a drawn depth within 0.45 tie levels. | Dynamapper | `LIGHT_SCENE_DEPTH_TOLERANCE_LEVELS` (`sh/art/light_scene.wesl`) | Test `gpu_pick_owner_order_matches_sorted_sprite_and_ground_instances` |

Not copied: the KR client weighs a row as 35.4 priority z (a static that much higher draws over a nearer one), and CUO
lets z cross rows above 100 z per step. Changing `PRIORITY_Z_STEPS_PER_ROW` to 35.4 would follow KR; there is no
setting.

Open items (full list in [DEPTH_SCALE_AND_KR_CLIENT.md § Open items](DEPTH_SCALE_AND_KR_CLIENT.md#open-items)):

- the land's depth export turns off its early depth test: KR-like costs 3.5 to 4 % on land views, 2 % in a city, over the geometric scale. Rerun the measure with [`frame_benchmark.py`](../../../skills/renderer-reference-validation/SKILL.md#frame-time-benchmark), not a test;
- a moved cap still loses to a static rising over the wall's top in the farther row; a wall with no floor on its top keeps a hairline where a perspective ray meets a farther floor at another z;
- the player cube keeps its rasterised depth; a free perspective camera is row-dominant only near (1, 1, 1); statics beyond 4 ranks share the last tie level.

## 4. Surfaces and floors

| Rule | Source | Code | Guard |
|---|---|---|---|
| Surface-like: an item authored SurfaceLike, with the Surface flag, or rendered wet (CC art: the Wet flag; EC art: the water shader where the item has an EC material). Surface-like items without Bridge take the `SurfaceLikeFloor` depth class; Bridge is walkable volume and stays Regular. | CC tiledata, EC tileart | `static_tile_is_surface_like`, `static_tile_renders_wet`, `resolve_static_depth_class` (`art/statics_collect.rs`) | Tests `depth_class_promotes_surface_like_floor_tiles`, `bridge_stone_and_stairs_retain_volume_depth_and_height_priority`, `wet_surface_like_tiles_keep_surface_depth_and_land_redirection`; fixture `britain-stacked-stairs` |
| A floor's top for caves and wall caps: a Surface or Bridge, not a Roof nor impassable; z plus its height, half of it for a Bridge. | Dynamapper | `surface_top` (`world/caves.rs`), `client_shadow_surface_tops` | Test `surface_tops_skip_roofs_and_impassables_and_halve_bridges` |
| Ground patches fade into the map at their open sides while the EC transitions are on, in every style that draws Enhanced art (or Classic art under Enhanced land). A patch is a height-0 Rotated item at the map's height with a world-UV base, and not Bridge, Wet, Foliage, Roof, NoDraw or Impassable (the lava rims draw their whole cell). A side is closed by the same patch at its Z, or by a Wall, Surface, Door or Transparent static (signposts, banners, rails) reaching its Z, or where the land beside it meets the patch cell's land with a hard edge (the upper EC material's coverage exponent 0, from the land source drawn: wooden floors of doorways and houses). | Dynamapper; KR Minoc yard (`Schermata_20261114_072854`), KR lava trench (`073402`) | `source_key`, `ClosingStatics`, `CLOSING_STATIC_FLAGS`, `land_terrain` (`art/static_surface.rs`); `EcTerrainParams::meets_with_hard_edge` (`render/terrain/coverage.rs`); `surface_blend.wesl` | Tests `walls_and_floors_beside_a_patch_close_its_side`, `doors_gates_and_signposts_beside_a_patch_close_its_side`, `impassable_lava_rims_are_not_patches`, `floor_land_meeting_a_patch_with_a_hard_edge_closes_its_side`, `the_upper_material_decides_whether_an_edge_is_hard`; fixtures `minoc-signpost-cobblestones`, `minoc-yard-floor-land-edges`, `minoc-hall-wall-feet-floor-land`, `lava-rim-crust` |

### Show Only the Player's Floor (z-view)

| | Classic, Enhanced | KR-like, UOD-Painterly |
|---|---|---|
| Rule | CUO's: a ceiling over the player (14 z rise, not Foliage or Transparent, Roof only with Surface) caps statics at its z and hides roofs; a roof on (x + 1, y + 1) caps at its connected roof's lowest z; land 16 z over the player caps land and statics at player z + 16. | `player_floor_rule`, default `kr`: the same ceiling test hides the 20-z layers above the player's, counted from the player's cell's averaged land z; a roof counts from its own cell; land goes whole while the player is below it. `classicuo` selects CUO's rule. |
| Source | CUO `GameScene.UpdateMaxDrawZ`, `Map.CalculateNearZ` | KR client 0x0043c350, 0x0043c5b0, 0x0043c7d0; diverges by counting from the player's cell (user, 44e4212a) |
| Code | `compute_z_view_cut`, `near_roof_z` (`world/z_view.rs`) | `compute_kr_z_view_cut`, `kr_layer_cut_z`, `kr_roof_is_hidden`; `UniformState::effective_player_floor_rule` |
| Guard | Tests `room_under_an_upper_floor_caps_at_the_floor`, `roof_only_caps_at_the_eaves_of_the_connected_roof`, `low_roof_cap_is_raised_to_the_headroom`, `cave_under_land_cuts_land_and_statics_at_the_headroom` | Tests `kr_ceiling_hides_the_layers_above_the_players`, `kr_roofs_count_from_their_own_cells`, `kr_layers_count_from_the_players_cell`, `kr_player_below_the_land_hides_all_land` |
| Fixtures | `britain-house-z50-zview-roof-ridge`, `minoc-hall-tables-zview`, `keep-inside-roofs-drawn` | `britain-house-z30-zview-kr-rule`, `britain-house-e20-zview-slate-roof`, `trinsic-low-roof-zview` |

### Caves (every style)

| Rule | Source | Code | Guard |
|---|---|---|---|
| A buried floor: a floor top at least 16 z (`CAVE_MIN_BURY_DEPTH`) under its cell's land. Outside caves it is hidden only under land that is drawn: land over the z-view's cut or NoDraw land covers nothing. | Dynamapper (66f50308, ecafc8d5) | `is_buried_floor`, `cave_floor` (`world/caves.rs`); `covering_land_z`, `TileAtlas::land_is_nodraw` | Tests `floors_buried_under_their_land`, `only_drawn_land_covers_a_buried_floor` |
| Caves drawn over the map (`world_rendering.caves`): buried floors connected to a mouth by walking steps (2 z) show through the land; basements (stairs) and sealed floors stay hidden. | Dynamapper | `CaveMap`, `CaveMap::floor_z`, `WALK_STEP_Z` | Tests `cave_with_a_mouth_shows`, `basement_stays_hidden`, `sealed_floor_stays_hidden`, `cave_map_hides_and_lifts`; fixtures `minoc-mine-entrance-zview-cave`, `minoc-mine-inside-zview-cave` |
| The player's cave: with the z-view on and the player on a buried floor, the cave (one 20-z level, walked to) is drawn whole with its rock walls, in place of both z-view rules. Outside it: dimmed (default) under CUO's cut, or black as the KR client shows a cave. | KR client (black); user decisions 0e0839cd, 72c19f3f, 1331d0bb | `PlayerCave`, `CaveSurroundings`, `player_cave_z_view_cut` | Tests `the_players_cave_is_the_floors_walked_to`, `the_players_cave_keeps_to_one_level`, `the_players_cave_draws_its_rock_walls`, `black_surroundings_hide_what_is_outside_the_players_cave`; fixtures `minoc-mine-mouth-player-cave-dimmed`, `minoc-mine-mouth-player-cave-black`, `yew-crypt-player-cave-dimmed`, `yew-crypt-player-cave-black` |
| Mobiles take the caves' static rules at their cell and z: one on a cave's floor (the player on a mine floor, mounted or not) is lifted with the floor, which would otherwise cover it whole; one outside the player's cave is dimmed with what is around it, and one the player's cave would hide still draws, dimmed. | Dynamapper (player-floors fix, 2026-11-26) | `CavesRes::mobile_view`, `cave_instance_flags` (`static_batches.rs`), `mobile_cave_flags` (`mobiles/draw.rs`) | Tests `a_mobile_on_a_caves_floor_is_lifted_with_it`, `a_mobile_takes_the_cave_bits_of_where_it_stands`. No fixture: the harness cannot show the player body (`core.toml` is not among its overrides) |
| Outside the player's cave, land and statics keep `CAVE_OUTSIDE_SHADE` (0.3) of their brightness and `CAVE_OUTSIDE_SATURATION` (0.6) of their colour. | user, 2026-11-16 (1331d0bb) | `cave_outside_tone`, `STATIC_FLAG_CAVE_OUTSIDE` (`depth.wesl`) | Tests `the_cave_outside_tone_is_the_rules`, `gpu_cave_outside_tone_keeps_the_rules_share_of_brightness_and_colour`; fixture `minoc-mine-mouth-player-cave-dimmed` |
| The cave mouth is the cave's: the land rising from an exit over the floor is drawn whole, undimmed, in front of the floor; what stands on the exit (the entrance arches) is lifted with the cave. A floor static on the land's face makes it a doorway, not a mouth. | Dynamapper (a7de9a29, 71afddf7) | `PlayerCave::owns_land` | Test `the_players_cave_owns_its_mouth`; fixtures `britain-north-cave-arch-player-cave`, `covetous-west-cave-arch-player-cave`, `shame-coast-cave-arch-player-cave`, `pentagram-alcove-player-cave` |
| The mouth's black opening is see-through: the cave mouth veil draws it over what lies behind at `CAVE_MOUTH_OPENING_OPACITY` (0.65) at the land cut, 0.35 toward the cave and 0.9 toward the outside over 12 z, feathered over half a tile at its border. | user decisions (40369f15, b85fa051) | `cave_mouth_opening`, `cave_mouth_tunnel_opacity`, `cave_mouth_feather` (`sh/land/main.wesl`); `terrain/cave_mouth_veil.rs` | Fixtures `minoc-mine-mouth-opening-see-through-perspective`, `minoc-north-cave-mouth-opening-see-through-close`, `britain-north-cave-arch-mouth-edge-player-cave` |
| Classic darkness key: the veil's opacity ramps from black to rock between luminance 0.006 and 0.02, so a Classic frame texmap's rock stays solid around a see-through opening (its opening is at most 0.0044, its rock from 0.02); EC and KR frames are black throughout. | measured on the Classic texmaps | `cave_mouth_veil_opacity`, `CAVE_MOUTH_KEY_BLACK_LUMINANCE`, `CAVE_MOUTH_KEY_ROCK_LUMINANCE` | Tests `the_darkness_key_lies_between_a_classic_frames_opening_and_its_rock`, `classic_cave_exit_frames_paint_their_opening_and_rock_apart` (packages); fixture `minoc-mine-mouth-player-cave-dimmed` (cc/cc in Classic). The EC and KR frames' black is not tested |
| Backdrop: under each veil the same quads draw black at the far plane first, so the opening shows black, not the clear colour, where nothing lies behind it. | Dynamapper (b85fa051) | `backdrop_material` (`cave_mouth_veil.rs`), `CAVE_MOUTH_BACKDROP_MODE` (`sh/land/main.wesl`) | Test `the_veil_sorts_last_and_its_black_backdrop_first`; fixture `minoc-north-cave-mouth-opening-see-through-close` |

Details: [`LAND_TEXTURES_AND_TRANSITIONS.md`](LAND_TEXTURES_AND_TRANSITIONS.md#ec-ownership-and-routing) and the module
comments of `world/caves.rs` and `world/z_view.rs`.

## 5. Land

| Rule | Source | Code | Guard |
|---|---|---|---|
| Each land id is drawn from the selected land source, else the other EC-layout source (EC and KR stand in for each other), else Classic (its texmap, else its land art as the CC client does); an id no source has draws the magenta and black checker. Classic selected never borrows EC-layout terrain. | user, 2026-11-12; CC `LandView.Draw` | `dynamapper/src/render/assets/land/routing.rs` | Tests `a_missing_id_comes_from_the_other_enhanced_source_then_classic`, `classic_land_without_a_texmap_draws_its_art`, `classic_never_borrows_enhanced_terrain`, `nothing_has_it_draws_the_missing_pattern` |
| The route file (`LandRoutes.kdl`) and its profiles stand in front of that chain; a route its source cannot draw falls back with a warning. | user | `content::land_routes`, `routing.rs` | Tests `a_route_draws_the_id_it_names_from_its_source`, `a_route_its_source_cannot_draw_falls_back_with_a_warning`, `classic_selected_skips_enhanced_routes` |
| EC land routes Classic ids through `legacyterrainmap.csv` and the `TerrainTranscode.kdl` overlay embedded in `tex_land_ec.uddp`. KR land routes the ids measured on KR's facets. | EC `GameData.uop`; KR facets | `conv/tex_land_kr.rs` (`KrLandMap`) | Tests `kingdom_reborn_routes_replace_the_csv_ones_and_leave_the_routing_table` |
| The KR land is an overlay over the EC land: only the textures that differ from the EC client's by more than `KR_IMAGE_EQUIVALENCE_THRESHOLD` (8 levels) are packed; the rest are the EC land's slots. | 0b308d79 | `TexLandEcPackage::kr_land`, `conv/tex_land_kr.rs` | Test `kingdom_reborn_definitions_come_first_and_the_enhanced_client_fills_the_rest`; fixture `painterly-kr-land-blur-layout` |
| Transitions follow the land source, not the style: EC blending on and CC transitions off by default (`graphics.ec_terrain_transitions`, `cc_terrain_transitions`). EC-layout cells blend through coverage; a border with Classic or missing cells is a hard edge. | user ([terrain transition controls](../../agent_notes/terrain_transition_controls.md)) | `SectGraphics::terrain_transition_flags`, `apply_render_preset`; `sh/land/coverage.wesl`, `transitions.wesl` | Tests `bundled_terrain_transitions_blend_ec_land_and_leave_cc_land_hard`, `terrain_transition_controls_round_trip_independently_of_source_and_presets`, `every_style_keeps_the_terrain_transitions_as_set`; fixture `minoc-river-water-terrain` |
| Classic transitions, when on, blend two dry Classic texmaps only where their facing edges' mean colours differ, so a hand-drawn transition texmap or a material's variant, painted to match, keeps its authored edge and same-texture ground stays exact. The neighbour is sampled from its own slot, mirrored across the shared edge, through a noise-broken boundary. Built surfaces (floors, flagstones, brick, by tiledata name), wet, liquid, black land, land art and missing cells keep hard edges; cobblestone roads blend. | user, 2026-11-26 (the old blend smeared every slot change into streaks) | `cc_texmap_edge_colors` (`assets/land.rs`), `is_built_land_name` (`assets/land/cache.rs`); `sh/land/cc_transitions.wesl` | Tests `cc_texmap_edge_colors_average_each_side_of_the_slot`, `built_land_names_are_floors_flagstones_and_brick`, `liquid_sampling_gpu_preserves_data_and_source_edges` (ignored, Vulkan); fixtures `britain-cc-terrain-transitions-paving`, `britain-cc-terrain-transitions-dirt-grass` |
| Black land (0x1AE, 0x1DB, 0x244) never blends, in any source. | KR client draws a cave floor's jagged edge against the opening | `is_black_land`, `cell_terrain_flags`, `TERRAIN_SHADER_FLAG_NEVER_BLENDS` (`assets/tile_atlas.rs`) | Test `only_black_land_never_blends`; fixtures `britain-northeast-hills-cave-mouth-player-cave`, `wrong-hills-cave-mouth-player-cave` |
| Cave exit frames (0x1D3–0x1DA, 0x7EC–0x7F1, 0x834–0x839, 0x8C1–0x8C6) never blend only on the player's cave's mouth; elsewhere they blend, so the opening fades softly into the rock. | EC/KR draw them all black (materials 44, 45, 154); user (031e3dec) | `is_cave_exit_frame`, `cell_terrain_flags` | Tests `cave_exit_frames_are_not_black_land`, `cave_exit_frames_never_blend_only_on_the_player_caves_mouth`; fixtures `minoc-mine-mouth-outside`, `britain-north-cave-arch-outside` |
| Missing EC terrain keeps the explicit missing-texture path; no silent substitution of Classic texmaps. | [ENGINEERING.md](../../../ENGINEERING.md) | `routing.rs` | Test `a_missing_selected_package_names_what_is_shown_instead` |
| The KR material grain (UOD-Painterly's micro-contrast, KR profile) and the land blur's per-tile rotation are seeded by world position alone, never by atlas page, layer or texture payload. | d49b3366, 84b61085 | `apply_kr_material_micro_contrast`, `KR_MATERIAL_GRAIN_CELLS_PER_TILE` (`sh/land/main.wesl`); `rand01_from_tile` (`sh/land/sampling.wesl`) | Tests `the_kr_land_grain_is_seeded_by_the_world_position_alone`, `the_land_blur_rotation_is_seeded_by_the_world_tile_alone` (they read the shader source: the land shader does not compose in a test); fixture `painterly-kr-land-blur-layout` |
| Land texture variation breaks up listed materials' repetition in Enhanced, KR-like and UOD-Painterly; Classic draws no EC materials. | user (279ee9d3, 085e2b6c) | `preset_group_has_land_texture_variation` | Test `bundled_presets_turn_the_land_texture_variation_on_in_every_style_but_classic`; fixture `yew-forest-canopy` |

Details: [`LAND_TEXTURES_AND_TRANSITIONS.md`](LAND_TEXTURES_AND_TRANSITIONS.md).

## 6. Lights

| | Classic | Enhanced, KR-like | UOD-Painterly |
|---|---|---|---|
| Dimming | `(32 − L) / 32` + lights × 31/32 (`classic_composite`, `ClientNightCompositeUniform::classic`); the night and the caves keep the afternoon's light colour, tint, ambient and sun, so the rule alone sets their darkness (unlit ground 0.625 and 0.19 of the afternoon) | `clamp(4 (0.25 level + B (1 − level)) scene)`, level = 1 − L/31, B = light sprites over the night map | the same composite at `overlay_strength` 0, then the sprites added once more as a glow (`light_sprite_glow`) |
| Source | CC client 0x00465600, 0x00448c80, 0x004571b0; CUO `IsometricLevel` | KR client `UOLightingPostProcessEffect::BuildCompositeQuad` 0x0041f5c0 (§1, P3.3) | user (UOD-Painterly look) |
| Lights | light.mul decals (`lighting/static_lights.rs`) from `world_lights.uddp`, signed | tileart block 3 light sprites (`lighting/client_light_sprites.rs`); LightSource alone gives no light | as Enhanced/KR-like |

| Rule | Source | Code | Guard |
|---|---|---|---|
| Light levels: 0 by day (the composite is the identity and does not run), 12 at night, 26 in caves; capped at 30. The level alone sets the night's blue: the KR screenshots' purple floors are a level of about 22, not a client tint. | ServUO's and RunUO's night and dungeon levels (night 12 since 2026-11-18, 10 before; user); KR client `OnGlobalLightLevel` 0x0056e8a0 | `MAX_CLIENT_LIGHT_LEVEL`, `CLIENT_LIGHT_LEVEL_DIVISOR` (`lighting/client_night.rs`) | Tests `full_day_is_the_identity`, `bundled_presets_dim_every_style_by_the_light_level`; fixtures `britain-lamp-posts-night-darkness`, `deceit-cave-candles-darkness`, `britain-slate-roofs-kr-night-level-22` |
| light.mul values are signed (−31..31): a negative pixel deepens the darkness, white whatever the light's colour. | CC client 0x00448c80 | `STATIC_LIGHT_TEXTURE_FORMAT` = `Rgba8Snorm` (`lighting/static_lights.rs`) | Tests `classic_lights_take_their_value_off_the_darkness`, `a_light_texture_signs_its_darkening_pixels_whatever_the_hue` |
| A light.mul light is its image at its pixel size on the screen, centred on its cell's centre at its z: a tile's diamond, 44 by 44 pixels, is √2 world units of the camera-facing quad across and √(2/3) up; the quad's axes are the screen's right and up, so the image is not mirrored. | CC client 0x00457f70 (the image's rect, its width and height from lightidx, centred 21 pixels below the cell's top vertex at its z) | `STATIC_LIGHT_BILLBOARD_WORLD_PER_TILE`, `static_light_quad_scale` (`lighting/static_lights.rs`) | Tests `a_tile_sized_light_covers_a_tile_diamond_on_screen`, `the_light_image_is_not_mirrored_on_screen`; fixtures `deceit-wall-torches-classic-light-level-16`, `britain-lamp-posts-night-darkness` |
| Light sprites stand at the EC base point (x + 0.5, y + 1.5) at K × light_scale world units per pixel, moved by the tileart offsets. | KR client P3.3 ([light sprite trace](../../agent_notes/light_sprite_proposal.md)) | `lighting/client_light_sprites.rs` | Tests `the_base_point_is_the_ec_static_anchor`, `the_lantern_light_has_the_clients_bounds` |
| Centre on items (`centre_on_items`, on by default): a sprite moves onto its item's flame, else onto its Classic art box; the tileart's default offsets that throw it more than 4.5 tiles away are dropped. | user, 2026-11-14 (client bug not copied) | `client_light_sprite_centred`, `art_flame_centroid`, `CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH` | Tests `a_sconces_light_moves_onto_its_flame`, `the_default_offsets_put_the_light_on_the_flame`, `without_a_flame_the_light_moves_onto_its_art`; fixtures `britain-candelabra-light-centring`, `britain-torch-walkway`, `britain-lamp-post` |
| `LightScale.kdl` scales chosen items' sprites (the lava 1.5 ×) in the sprite styles; Classic draws every light as its client does. | user (3ad6fc2e) | `lighting::light_scale` | Test `the_bundled_file_reads_and_scales_the_lava`; fixture `lava-dirt-local-lights` |

### What blocks a light

| `light_cover` | Rule | Code | Guard |
|---|---|---|---|
| `uod` (default, every style) | No light is hidden for being covered; a light goes only with its item (at or above the z limit). Each glow is clipped per pixel to the statics its light reaches, below its ceiling (lowest Surface or Roof ≥ 5 z above it). | `static_light_ceiling_z`, `lighting/light_scene.rs` (`render_light_scene`), `light_reaches` (`sh/art/light_clip.wesl`) | Test `a_lights_ceiling_is_the_lowest_floor_or_roof_over_it`; fixtures `britain-windows-light-cover`, `keep-z50-zview-lights` |
| `classicuo` | Hidden when its cover cell (x + 1, y + 1) holds a drawn static, not Transparent or NoDraw, ≥ 5 z above it and below the z limit. Source: CUO `GameScene.AddLight`. | `LIGHT_COVER_MIN_RISE`, `static_light_is_hidden` | Tests `a_roof_or_upper_floor_in_front_covers_a_room_light`, `see_through_and_never_drawn_statics_do_not_cover` |
| `clients` | Classic and Enhanced: the EC client's unclamped 20-z bands from the light cell's land (EC 0x00449f60). KR-like and UOD-Painterly: KR's 16 clamped layers, each from its own cell's averaged land (KR 0x004391d0). Both also hide a light under a floor or roof ≥ 5 z above it, which the clients' bands miss. | `LightCoverRule::of_style`, `client_light_is_hidden`, `kr_light_is_hidden`, `floor_covers_light` | Tests `clients_is_the_kr_rule_only_in_kr_like`, `the_clients_hide_a_light_under_a_higher_band`, `kr_counts_each_layer_from_its_own_cells_land`, `floors_and_roofs_over_a_light_cover_it_whatever_the_client_bands`; fixture `britain-door-view` |

- Every rule takes the view's z limit: a static the limit cuts no longer covers. Multi parts cover as statics do.
  Tests `the_z_limit_uncovers_lights_and_hides_its_own_cut`, `multi_parts_land_on_their_cells_at_their_expanded_z`;
  fixture `keep-z72-zview-lights`.
- In the player's cave a light goes with its static; outside the cave the lights, Classic's and the sprites alike,
  keep `CAVE_OUTSIDE_SATURATION` of their colour (greyed with the scene, 7fbbf27b). Code `CavesRes::light_view`.
  Test `lights_follow_the_players_cave`; fixture `minoc-mine-mouth-player-cave-dimmed`.

Details: [`CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`](CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md) (night composite, Classic
night lighting, light sprites, lights under a roof or a floor, KR static shadows).

## 7. Tile data

The tile data a static is drawn and picked by follows the art it is drawn from, not the style
(`drawn_tile_data`, `render/statics/statics_collect.rs`; `TileMetaPackageRes::for_art`, `dynamapper/src/content.rs`).

| Drawn from | Tables | Note |
|---|---|---|
| CC art (any style) | the clients' table: Classic tiledata flags merged with the EC tileart flags, without the overrides (`TileMetaPackage::without_overrides`, rebuilt from `metadata/overrides.bin`) | the overrides were measured on EC/KR art only |
| EC art (any style) | the EC tables with `TilemetaOverrides.kdl` applied | |
| KR art (statics continuing the KR land, any style) | the KR section of `tilemeta.uddp` (`kr_tilemeta`) | KR's NoDraw, Surface and Rotated differ on some of them |

- Source: user decisions 2026-11-16 and 2026-11-17 (a854bb51, 8e214282, 89c173e6). Tests `statics_drawn_with_classic_art_keep_the_clients_tile_data`,
  `the_overrides_are_undone_from_their_record` (udd-assets), `kingdom_reborn_kinds_read_the_kingdom_reborn_records`.

**The overrides** ([`dynamapper/assets/cc_ec_convtables/TilemetaOverrides.kdl`](../../../dynamapper/assets/cc_ec_convtables/TilemetaOverrides.kdl)),
applied when `tilemeta.uddp` is packed, each from a KR-values comparison of 2026-11-17:

| Items | Change | Fixture |
|---|---|---|
| 230 lava edge pieces | add Surface (KR sets it, EC dropped it) | `lava-dirt-local-lights` |
| swamp pieces | add Surface | `deceit-swamp-surface`, `swamp-statics` |
| swamp 0x3236–0x323F by Deceit | add Surface and Rotated (KR's camera quad) | `deceit-swamp-rotated`, `deceit-swamp-rotated-perspective` |
| ankh of sacrifice 0x1D97, 0x1D98 | add LightSource and KR's light sprite (`kr-light`) | `ankh-of-sacrifice-light` |

Code: `lib/udd-assets/src/overrides/tilemeta_overrides.rs` (strict KDL), `lib/udd-conv/src/metadata/tilemeta_overrides.rs`.
Tests `the_bundled_file_parses`, `applies_flags_and_kr_lights_and_records_what_it_replaced`, `rejects_what_it_does_not_know`.

**KR delta threshold.** A KR texture counts as the EC one re-encoded, and is not packed, when it differs by at most
`KR_SPRITE_EQUIVALENCE_THRESHOLD` (17 levels, `visible_difference`, measured through the windows the items draw) for
sprites, or `KR_IMAGE_EQUIVALENCE_THRESHOLD` (8, `image_difference`, whole image) for terrain, material, land and light
textures. Code `conv/kr_items.rs` (`compare_world_art`, `TextureUse::threshold`). Test
`the_thresholds_part_the_measured_noise_from_the_measured_changes`, `a_sprite_is_measured_through_its_window_alone`.

**Open item.** The z-view (`sys_update_z_view_limit`) and the cave analyses (`world/caves.rs`) read the overridden EC
tables (`TileMetaPackageRes::for_world_analyses`) whatever the art source. No visible effect has been found. Test
`the_world_analyses_read_the_overridden_tables_whatever_the_art` pins the current behaviour and changes with the item.

## 8. Client bugs we don't copy

| Client behaviour | Decision | What we do | Code | Guard |
|---|---|---|---|---|
| KR and EC lights shown off their item: the tileart's default offsets ((−5, −4) on 378 items) throw the glow 4.78 tiles or more away. | user, 2026-11-14 | centre on items, on by default | `client_light_sprite_centred` | Fixture `britain-candelabra-light-centring` |
| Lights through floors and roofs: the clients draw light sprites with no depth test, and their bands keep a light under a floor just above it in its band. | user, 2026-11-14 | `uod` light cover by default; `clients` also applies `floor_covers_light` | `lighting/light_scene.rs`, `floor_covers_light` | Fixtures `britain-windows-light-cover`, `keep-z50-zview-lights` |
| Particles through floors (KR). | user, 2026-11-14 | no rule yet: the world view draws no particle effects | none | none: nothing is drawn to guard |
| CC client's coloured lights read their colour table before its start at a negative light.mul value, brightening the pixels the white lights darken. | 2026-11-16 (857e4c1f) | negative values darken, whatever the colour | `lighting/static_lights.rs` | Test `a_light_texture_signs_its_darkening_pixels_whatever_the_hue` |
| KR counts each object's height layer from its own cell's land, dropping whole walls of a building on uneven land. | user (44e4212a) | layers counted from the player's cell; roofs keep KR's own count | `kr_layer_cut_z` | Test `kr_layers_count_from_the_players_cell`; fixture `britain-house-z30-zview-kr-rule` |
| EC tileart data: copied legacy windows that cut 215 statuettes; Surface dropped from lava and swamp pieces; the ankh's light missing. | user, 2026-11-17 (70e54459, 8e214282) | KR's windows adopted; `TilemetaOverrides.kdl` | `adopt_kr_world_windows`, overrides | Section 2 and 7 guards |

Not reproduced by choice (not bugs): KR's 35.4 priority z per row and CUO's 100 (section 3); the CC client's
DarkNights option, its 5-bit quantisation and the personal light level
([Classic night lighting](CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md)).
