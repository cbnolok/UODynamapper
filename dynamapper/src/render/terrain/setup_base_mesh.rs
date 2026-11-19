use super::draw_mesh::{ChunkScale, LandMeshHandles, LandMeshLod};
use bevy::{
    prelude::*,
    mesh::Indices,
    render::render_resource::PrimitiveTopology,
    asset::RenderAssetUsages,
};
use crate::util_lib::tracked_plugin::log_system_add_startup;
use crate::app::system_sets::StartupSysSet;
use crate::fname;

/// A grid quad's two triangles, by its corners: 0 at its origin, 1 one step along x, 2 one step along z, 3 both. The
/// diagonal runs from 0 to 3; the cave mouth veil's quads (`build_cell_quads_mesh`) cut theirs the same way, so they
/// lie exactly over the chunk mesh's.
const QUAD_TRIANGLES: [u32; 6] = [0, 3, 1, 0, 2, 3];

fn build_chunk_mesh(chunk_tiles: usize, step_tiles: usize) -> Mesh {
    let core_w: usize = chunk_tiles;
    let core_h: usize = chunk_tiles;

    assert!(step_tiles > 0);
    assert!(core_w.is_multiple_of(step_tiles));
    assert!(core_h.is_multiple_of(step_tiles));

    let quads_x = core_w / step_tiles;
    let quads_y = core_h / step_tiles;
    let grid_w = quads_x + 1;
    let grid_h = quads_y + 1;

    let estimated_vertex_count = grid_w * grid_h;
    let mut positions = Vec::with_capacity(estimated_vertex_count);
    let mut uvs = Vec::with_capacity(estimated_vertex_count);
    let mut indices = Vec::with_capacity(quads_x * quads_y * 6);

    // Create a flat grid at y=0. Vertex shader displaces y from tile metadata.
    // UV_0 remains normalized over the full 8x8 chunk so texture mapping is stable across LODs.
    for gy in 0..=quads_y {
        for gx in 0..=quads_x {
            let x = (gx * step_tiles) as f32;
            let z = (gy * step_tiles) as f32;
            positions.push([x, 0.0, z]);
            uvs.push([x / core_w as f32, z / core_h as f32]);
        }
    }

    for ty in 0..quads_y {
        for tx in 0..quads_x {
            let v0 = (ty * grid_w + tx) as u32;
            let v1 = v0 + 1;
            let v2 = ((ty + 1) * grid_w + tx) as u32;
            let v3 = v2 + 1;
            let corners = [v0, v1, v2, v3];
            indices.extend(QUAD_TRIANGLES.map(|corner| corners[corner as usize]));
        }
    }

    grid_mesh(positions, uvs, indices)
}

/// A mesh of the quads of a chunk mesh with `span` tiles a side and `step` tiles a quad, each quad named by its origin
/// in the chunk, a multiple of `step`: the same vertices and triangles as the chunk mesh's there (`build_chunk_mesh`),
/// which the land vertex shader lifts the same.
pub(super) fn build_cell_quads_mesh(span: u32, step: u32, quads: impl ExactSizeIterator<Item = UVec2>) -> Mesh {
    let mut positions = Vec::with_capacity(quads.len() * 4);
    let mut uvs = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * QUAD_TRIANGLES.len());
    for quad in quads {
        let first = positions.len() as u32;
        for corner in [UVec2::ZERO, UVec2::X, UVec2::Y, UVec2::ONE] {
            let (x, z) = ((quad.x + corner.x * step) as f32, (quad.y + corner.y * step) as f32);
            positions.push([x, 0.0, z]);
            uvs.push([x / span as f32, z / span as f32]);
        }
        indices.extend(QUAD_TRIANGLES.map(|corner| first + corner));
    }
    grid_mesh(positions, uvs, indices)
}

/// The land mesh of a flat grid's `positions`, their `uvs` and the triangles `indices`.
fn grid_mesh(positions: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, indices: Vec<u32>) -> Mesh {
    let dummy_normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let dummy_uv1s = vec![[0.0, 0.0]; positions.len()];

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, dummy_normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, dummy_uv1s);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// This startup system generates shared terrain meshes for multiple LODs and scales.
pub fn setup_land_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    log_system_add_startup::<super::DrawLandChunkMeshPlugin>(StartupSysSet::SetupSceneStage1, fname!());
    // Standard 32×32 tile meshes at three vertex-density LODs:
    let high = meshes.add(build_chunk_mesh(32, 1));   // 1089 verts (zoom < 4)
    let medium = meshes.add(build_chunk_mesh(32, 2)); // 289 verts  (zoom 4–10)
    let low = meshes.add(build_chunk_mesh(32, 4));    // 81 verts   (zoom 10+ fallback)
    // Wide meshes for reduced entity count at high zoom:
    let wide16 = meshes.add(build_chunk_mesh(64, 2)); // 1089 verts, covers 64×64 tiles (zoom 10–25)
    let wide32 = meshes.add(build_chunk_mesh(128, 4)); // 1089 verts, covers 128×128 tiles (zoom 25–50)
    let wide64 = meshes.add(build_chunk_mesh(256, 8)); // 1089 verts, covers 256×256 tiles (zoom ≥50)
    let wide128 = meshes.add(build_chunk_mesh(512, 16)); // 1089 verts, covers 512×512 tiles (extreme zoom-out)
    let wide256 = meshes.add(build_chunk_mesh(1024, 32)); // 1089 verts, covers 1024×1024 tiles (maximum zoom-out)

    commands.insert_resource(LandMeshHandles {
        high,
        medium,
        low,
        wide16,
        wide32,
        wide64,
        wide128,
        wide256,
    });
    commands.insert_resource(LandMeshLod::default());
    commands.insert_resource(ChunkScale::default());
}
