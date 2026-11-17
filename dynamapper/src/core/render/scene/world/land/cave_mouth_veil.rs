//! The black opening of the player's cave's mouth drawn see-through (caves.rs `PlayerCave`): from inside the cave the
//! opening, the black land and the black cave exit frames over its mouth (land/main.wesl `cave_mouth_opening`), stood
//! as an opaque black slab over the cave's floor and the land outside behind it. The land leaves it out, and the veil,
//! the land's material drawn blended (`veil_material`) over the mouth's quads alone (`sys_sync_cave_mouth_veils`),
//! draws it over what lies behind, at land/main.wesl's `CAVE_MOUTH_OPENING_OPACITY`.
//!
//! The veil's quads are those of the chunk meshes under the mouth, the same vertices and triangles
//! (setup_base_mesh.rs `build_cell_quads_mesh`), drawn as children of their chunks: the land vertex shader lifts them
//! the same, and the land fragment shader gives them the land's depth. They draw after everything else (`VEIL_SORT_BIAS`)
//! with the depth test: the opening darkens what lies behind it and leaves what stands in front of it, the arches.

use super::draw_mesh::{LandMeshHandles, SharedLandMaterial};
use super::mesh_material::LandCustomMeshMaterial;
use super::setup_base_mesh::build_cell_quads_mesh;
use super::{LCMesh, TILE_NUM_PER_CHUNK_DIM};
use crate::core::render::scene::world::caves::CavesRes;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::prelude::*;
use std::collections::BTreeSet;

/// The veil's sort distance bias in the transparent phase, in view units: more than any view's depth, so it draws
/// after every blended static and shadow, which then lie behind it darkened as the opaque ones do. The land material
/// clears it from the pipeline's depth bias (`LandMaterialExtension::specialize`): it only orders the draw.
const VEIL_SORT_BIAS: f32 = 1.0e6;

/// The cave mouth veil's material, the land's (`SharedLandMaterial`) drawn blended.
#[derive(Resource)]
pub struct CaveMouthVeilMaterial(Handle<LandCustomMeshMaterial>);

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
    veil.extension.scene_uniform.cave_mouth_veil = 1.0;
    veil
}

/// Keeps the veil's material a blended copy of the land's, made again whenever the land's changes.
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
    let Some(veil) = materials.get(&land_material.0).map(veil_material) else {
        return;
    };
    match veil_material_r {
        None => commands.insert_resource(CaveMouthVeilMaterial(materials.add(veil))),
        Some(veil_material_r) => {
            if let Some(mut current) = materials.get_mut(&veil_material_r.0) {
                *current = veil;
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
            Mesh3d(veil_mesh),
            MeshMaterial3d(veil_material.0.clone()),
            Transform::IDENTITY,
            // The vertex shader lifts the flat quads to the land: the chunk's bounds hold them.
            NoAutoAabb,
            *aabb,
            ChildOf(chunk),
        )).id();
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
}
