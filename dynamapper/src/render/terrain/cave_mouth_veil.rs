//! The black opening of the player's cave's mouth drawn see-through (caves.rs `PlayerCave`): from inside the cave the
//! opening, the black land and the black cave exit frames over its mouth (land/main.wesl `cave_mouth_opening`), stood
//! as an opaque black slab over the cave's floor and the land outside behind it. The land leaves it out, and the veil,
//! the land's material drawn blended (`veil_material`) over the mouth's quads alone (`sys_sync_cave_mouth_veils`),
//! draws it over what lies behind like a tunnel's dark, more opaque toward the outside and less toward the cave, fading
//! in from the opening's border (land/main.wesl `cave_mouth_veil_opacity`).
//!
//! The veil's quads are those of the chunk meshes under the mouth, the same vertices and triangles
//! (setup_base_mesh.rs `build_cell_quads_mesh`), drawn as children of their chunks: the land vertex shader lifts them
//! the same, and the land fragment shader gives them the land's depth. They draw after everything else (`VEIL_SORT_BIAS`)
//! with the depth test: the opening darkens what lies behind it and leaves what stands in front of it, the arches.
//!
//! Under each veil its backdrop, the same quads drawn in black at the far plane before every other blended draw
//! (`backdrop_material`), fills what nothing else draws behind the opening, so that the veil shows black there and not
//! the clear colour.

use super::draw_mesh::{LandMeshHandles, SharedLandMaterial};
use super::mesh_material::LandCustomMeshMaterial;
use super::setup_base_mesh::build_cell_quads_mesh;
use super::{LCMesh, TILE_NUM_PER_CHUNK_DIM};
use crate::world::caves::CavesRes;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::prelude::*;
use std::collections::BTreeSet;

/// The veil's sort distance bias in the transparent phase, in view units: more than any view's depth, so it draws
/// after every blended static and shadow, which then lie behind it darkened as the opaque ones do. The land material
/// clears it from the pipeline's depth bias (`LandMaterialExtension::specialize`): it only orders the draw.
const VEIL_SORT_BIAS: f32 = 1.0e6;
/// The backdrop's sort distance bias in the transparent phase: less than any view's depth, so it draws first, under
/// every blended static and shadow behind the opening.
const BACKDROP_SORT_BIAS: f32 = -VEIL_SORT_BIAS;
/// `SceneUniform::cave_mouth_veil` in the veil's material: the land shader draws the opening alone, blended.
const VEIL_SHADER_MODE: f32 = 1.0;
/// `SceneUniform::cave_mouth_veil` in the backdrop's material: the land shader draws the opening alone, black, at the
/// far plane.
const BACKDROP_SHADER_MODE: f32 = 2.0;

/// The cave mouth veil's materials: the land's (`SharedLandMaterial`) drawn blended, and its backdrop's.
#[derive(Resource)]
pub struct CaveMouthVeilMaterial {
    veil: Handle<LandCustomMeshMaterial>,
    backdrop: Handle<LandCustomMeshMaterial>,
}

/// A veil entity, child of the land chunk whose mouth quads it draws, built from the chunk's mesh `chunk_mesh`.
#[derive(Component)]
pub struct CaveMouthVeil {
    chunk_mesh: AssetId<Mesh>,
}

/// The land material `land` as the veil draws it: blended, sorted last, its shader drawing only the opening.
fn veil_material(land: &LandCustomMeshMaterial) -> LandCustomMeshMaterial {
    let mut veil = land.clone();
    veil.base.alpha_mode = AlphaMode::Blend;
    veil.base.depth_bias = VEIL_SORT_BIAS;
    veil.extension.scene_uniform.cave_mouth_veil = VEIL_SHADER_MODE;
    veil
}

/// The land material `land` as the veil's backdrop draws it: blended, sorted first, its shader drawing only the
/// opening, black, at the far plane, where the depth test leaves it only what nothing else drew.
fn backdrop_material(land: &LandCustomMeshMaterial) -> LandCustomMeshMaterial {
    let mut backdrop = land.clone();
    backdrop.base.alpha_mode = AlphaMode::Blend;
    backdrop.base.depth_bias = BACKDROP_SORT_BIAS;
    backdrop.extension.scene_uniform.cave_mouth_veil = BACKDROP_SHADER_MODE;
    backdrop
}

/// Keeps the veil's materials, a blended copy of the land's and its backdrop's, made again whenever the land's changes.
pub fn sys_sync_cave_mouth_veil_material(
    mut commands: Commands,
    land_material: Option<Res<SharedLandMaterial>>,
    veil_material_r: Option<Res<CaveMouthVeilMaterial>>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    mut events: MessageReader<AssetEvent<LandCustomMeshMaterial>>,
) {
    let Some(land_material) = land_material else {
        events.clear();
        return;
    };
    let land_changed = events.read().any(|event| event.is_modified(&land_material.0));
    if veil_material_r.is_some() && !land_changed {
        return;
    }
    let Some((veil, backdrop)) = materials.get(&land_material.0).map(|land| (veil_material(land), backdrop_material(land)))
    else {
        return;
    };
    match veil_material_r {
        None => commands.insert_resource(CaveMouthVeilMaterial {
            veil: materials.add(veil),
            backdrop: materials.add(backdrop),
        }),
        Some(veil_material_r) => {
            if let Some(mut current) = materials.get_mut(&veil_material_r.veil) {
                *current = veil;
            }
            if let Some(mut current) = materials.get_mut(&veil_material_r.backdrop) {
                *current = backdrop;
            }
        }
    }
}

/// The veil's quads in the chunk at (`gx`, `gy`) of the base chunk grid with `span` tiles a side and `step` tiles a quad:
/// the origins in the chunk of the quads holding a cell of the mouth's land `mouth`.
fn chunk_mouth_quads(gx: u32, gy: u32, span: u32, step: u32, mouth: &[(i32, i32)]) -> BTreeSet<(u32, u32)> {
    let origin = IVec2::new((gx * TILE_NUM_PER_CHUNK_DIM) as i32, (gy * TILE_NUM_PER_CHUNK_DIM) as i32);
    mouth.iter()
        .map(|&(x, y)| IVec2::new(x, y) - origin)
        .filter(|cell| cell.cmpge(IVec2::ZERO).all() && cell.cmplt(IVec2::splat(span as i32)).all())
        .map(|cell| (cell.x as u32 / step * step, cell.y as u32 / step * step))
        .collect()
}

/// Keeps a veil on each land chunk holding the player's cave's mouth, its quads over the mouth's land, and none
/// elsewhere: rebuilt when the player's cave changes or a chunk takes another mesh (its spawn, its level of detail).
pub fn sys_sync_cave_mouth_veils(
    mut commands: Commands,
    caves: Res<CavesRes>,
    veil_material: Option<Res<CaveMouthVeilMaterial>>,
    mesh_handles: Option<Res<LandMeshHandles>>,
    mut meshes: ResMut<Assets<Mesh>>,
    chunks: Query<(Entity, &LCMesh, &Mesh3d, &Aabb)>,
    changed_chunks: Query<(), (With<LCMesh>, Changed<Mesh3d>)>,
    veils: Query<(Entity, &ChildOf, &CaveMouthVeil)>,
    mut last_revision: Local<Option<u64>>,
) {
    let (Some(veil_material), Some(mesh_handles)) = (veil_material, mesh_handles) else {
        return;
    };
    let cave_changed = *last_revision != Some(caves.revision());
    if !cave_changed && changed_chunks.is_empty() {
        return;
    }
    *last_revision = Some(caves.revision());
    let mouth: Vec<(i32, i32)> = caves.player().map(|cave| cave.mouth_land_cells().collect()).unwrap_or_default();
    let mut kept = Vec::new();
    for (chunk, lc_mesh, mesh, aabb) in &chunks {
        let Some((span, step)) = mesh_handles.grid(mesh) else {
            continue;
        };
        let quads = chunk_mouth_quads(lc_mesh.gx, lc_mesh.gy, span, step, &mouth);
        if quads.is_empty() {
            continue;
        }
        // A veil built from this mesh for the same cave stays.
        let current = veils.iter().find(|(_, parent, veil)| parent.parent() == chunk && veil.chunk_mesh == mesh.id());
        if let Some((veil, ..)) = current.filter(|_| !cave_changed) {
            kept.push(veil);
            continue;
        }
        let veil_mesh = meshes.add(build_cell_quads_mesh(span, step, quads.into_iter().map(|(x, y)| UVec2::new(x, y))));
        let veil = commands.spawn((
            CaveMouthVeil { chunk_mesh: mesh.id() },
            Mesh3d(veil_mesh.clone()),
            MeshMaterial3d(veil_material.veil.clone()),
            Transform::IDENTITY,
            // The vertex shader lifts the flat quads to the land: the chunk's bounds hold them.
            NoAutoAabb,
            *aabb,
            ChildOf(chunk),
        )).id();
        // The backdrop goes with its veil.
        commands.spawn((
            Mesh3d(veil_mesh),
            MeshMaterial3d(veil_material.backdrop.clone()),
            Transform::IDENTITY,
            NoAutoAabb,
            *aabb,
            ChildOf(veil),
        ));
        kept.push(veil);
    }
    for (veil, ..) in &veils {
        if !kept.contains(&veil) {
            commands.entity(veil).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util_lib::wesl_source::{wesl_const, wesl_f32_const, wesl_fn_body};

    #[test]
    fn mouth_quads_lie_in_their_chunk_on_its_mesh_grid() {
        let base = TILE_NUM_PER_CHUNK_DIM as i32;
        // Two cells in chunk (1, 2), one in the next chunk to the east, one off to the north.
        let mouth = [(base + 2, 2 * base + 5), (base + 3, 2 * base + 5), (2 * base, 2 * base + 5), (base + 3, base)];
        assert_eq!(chunk_mouth_quads(1, 2, TILE_NUM_PER_CHUNK_DIM, 1, &mouth), BTreeSet::from([(2, 5), (3, 5)]));
        // Two tiles a quad: both cells in the quad at (2, 4).
        assert_eq!(chunk_mouth_quads(1, 2, TILE_NUM_PER_CHUNK_DIM, 2, &mouth), BTreeSet::from([(2, 4)]));
        // A wide chunk twice the span takes the cell to the east too.
        assert_eq!(chunk_mouth_quads(1, 2, 2 * TILE_NUM_PER_CHUNK_DIM, 2, &mouth),
            BTreeSet::from([(2, 4), (TILE_NUM_PER_CHUNK_DIM, 4)]));
        assert!(chunk_mouth_quads(1, 2, TILE_NUM_PER_CHUNK_DIM, 1, &[]).is_empty());
    }

    const LAND_MAIN_WESL: &str = include_str!("../../../assets/shaders/world/land/main.wesl");

    /// The veil draws after every blended static and shadow, its backdrop before them, whatever the view's depth; the
    /// land shader tells the two apart by the mode each material sets, the backdrop's black at the far plane.
    #[test]
    fn the_veil_sorts_last_and_its_black_backdrop_first() {
        // Reverse z: depth 0 is the far plane.
        const REVERSE_Z_FAR_DEPTH: f32 = 0.0;
        // The deepest view: the perspective camera's far plane (camera/perspective.rs `projection`), beyond the
        // orthographic one's (camera.rs ORTHO_FAR, 1000).
        const DEEPEST_VIEW: f32 = 20000.0;
        assert!(VEIL_SORT_BIAS > DEEPEST_VIEW && BACKDROP_SORT_BIAS < -DEEPEST_VIEW);
        assert!(VEIL_SHADER_MODE != 0.0 && BACKDROP_SHADER_MODE != 0.0 && VEIL_SHADER_MODE != BACKDROP_SHADER_MODE,
            "0 is the land's own material");
        assert_eq!(wesl_f32_const(LAND_MAIN_WESL, "CAVE_MOUTH_BACKDROP_MODE"), BACKDROP_SHADER_MODE);
        assert_eq!(wesl_f32_const(LAND_MAIN_WESL, "CAVE_MOUTH_BACKDROP_DEPTH"), REVERSE_Z_FAR_DEPTH);
        let fragment = wesl_fn_body(LAND_MAIN_WESL, "fragment");
        let backdrop = fragment.split("if (scene.cave_mouth_veil == CAVE_MOUTH_BACKDROP_MODE) {").nth(1)
            .and_then(|rest| rest.split("return out;").next())
            .expect("the land fragment draws the backdrop in its own branch");
        assert!(backdrop.contains("out.color = vec4<f32>(0.0, 0.0, 0.0,"), "the backdrop is black: {backdrop}");
        assert!(backdrop.contains("out.depth = CAVE_MOUTH_BACKDROP_DEPTH;"), "{backdrop}");
    }

    /// The highest linear luminance of the black opening of a Classic cave exit frame's texmap (STYLE_RULES §4, the
    /// Classic darkness key): black in 0x1D3-0x1DA, the darkest 5-bit grey, 0.0030, in 0x7EC-0x7F1 and 0x8C1-0x8C6,
    /// and 0.0044 in 0x834-0x839, measured 2026-11-18.
    const CLASSIC_FRAME_OPENING_MAX_LUMINANCE: f32 = 0.0045;
    /// The lowest linear luminance of the rock a Classic cave exit frame paints around its opening.
    const CLASSIC_FRAME_ROCK_MIN_LUMINANCE: f32 = 0.02;
    /// The key's luminance weights, linear sRGB's (Rec. 709).
    const KEY_LUMA_WEIGHTS: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

    /// The veil's darkness key splits a Classic frame's texmap where its painting does: its opening below the key's
    /// black, see-through, its rock above the key's rock, opaque.
    #[test]
    fn the_darkness_key_lies_between_a_classic_frames_opening_and_its_rock() {
        assert_eq!(wesl_const(LAND_MAIN_WESL, "CAVE_MOUTH_KEY_LUMA_WEIGHTS"),
            format!("vec3<f32>({:?}, {:?}, {:?})", KEY_LUMA_WEIGHTS.x, KEY_LUMA_WEIGHTS.y, KEY_LUMA_WEIGHTS.z));
        let black = wesl_f32_const(LAND_MAIN_WESL, "CAVE_MOUTH_KEY_BLACK_LUMINANCE");
        let rock = wesl_f32_const(LAND_MAIN_WESL, "CAVE_MOUTH_KEY_ROCK_LUMINANCE");
        assert!(CLASSIC_FRAME_OPENING_MAX_LUMINANCE < black && black < rock && rock <= CLASSIC_FRAME_ROCK_MIN_LUMINANCE,
            "opening {CLASSIC_FRAME_OPENING_MAX_LUMINANCE} < black {black} < rock {rock} <= frame rock \
            {CLASSIC_FRAME_ROCK_MIN_LUMINANCE}");
    }

    /// Share of a Classic frame texmap's texels that may lie between the opening and the rock: the antialiasing of
    /// their border, at most 0.016 (0x834) when measured.
    const CLASSIC_FRAME_MAX_IN_BETWEEN_SHARE: f64 = 0.02;
    /// Share of a frame's texels its opening covers, at the least.
    const CLASSIC_FRAME_MIN_OPENING_SHARE: f64 = 0.05;
    /// The land ids a map cell can name.
    const LAND_ID_COUNT: u32 = 0x4000;

    /// The measured side of `the_darkness_key_lies_between_a_classic_frames_opening_and_its_rock`: in the Classic
    /// texmaps of the cave exit frames (tile_atlas.rs `is_cave_exit_frame`) every texel is the opening, at most
    /// CLASSIC_FRAME_OPENING_MAX_LUMINANCE, or rock, at least CLASSIC_FRAME_ROCK_MIN_LUMINANCE, but for a few at
    /// their border.
    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn classic_cave_exit_frames_paint_their_opening_and_rock_apart() {
        let root = std::path::PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let tilemeta = udd_assets::tilemeta::TileMetaPackage::load(root.join("tilemeta.uddp")).unwrap();
        let texmaps = udd_assets::tex_land_cc::TexLandCcPackage::load(root.join("tex_land_cc.uddp")).unwrap();
        let mut frames = 0;
        for land_id in (0..LAND_ID_COUNT).filter(|&id| crate::render::assets::tile_atlas::is_cave_exit_frame(id)) {
            let Some(tile) = tilemeta.land_tile(land_id) else { continue };
            let Some(rgba) = texmaps.get_pixel_data(u32::from(tile.texture_id)) else { continue };
            // The key's luminance: the texture's linear colour (it is sampled sRGB) under the key's weights.
            let luminance = |texel: &[u8]| {
                let linear = Color::srgb_u8(texel[0], texel[1], texel[2]).to_linear();
                Vec3::new(linear.red, linear.green, linear.blue).dot(KEY_LUMA_WEIGHTS)
            };
            let texels = rgba.chunks_exact(4).map(luminance).collect::<Vec<_>>();
            let share = |count: usize| count as f64 / texels.len() as f64;
            let opening = share(texels.iter().filter(|&&l| l <= CLASSIC_FRAME_OPENING_MAX_LUMINANCE).count());
            let rock = share(texels.iter().filter(|&&l| l >= CLASSIC_FRAME_ROCK_MIN_LUMINANCE).count());
            let between = 1.0 - opening - rock;
            println!("land {land_id:#x} texmap {:#x}: opening {opening:.3}, rock {rock:.3}, between {between:.4}",
                tile.texture_id);
            assert!(between <= CLASSIC_FRAME_MAX_IN_BETWEEN_SHARE, "land {land_id:#x}: {between} between");
            assert!(opening >= CLASSIC_FRAME_MIN_OPENING_SHARE, "land {land_id:#x}: opening {opening}");
            frames += 1;
        }
        assert!(frames > 0, "no cave exit frame has a Classic texmap");
    }
}
