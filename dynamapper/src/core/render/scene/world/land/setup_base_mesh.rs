use super::{
    TILE_NUM_PER_CHUNK_DIM,
    draw_mesh::{LandMeshHandles, LandMeshLod},
};
use bevy::{
    prelude::*,
    mesh::Indices,
    render::render_resource::PrimitiveTopology,
    asset::RenderAssetUsages,
};

fn build_chunk_mesh(step_tiles: usize) -> Mesh {
    let core_w: usize = TILE_NUM_PER_CHUNK_DIM as usize;
    let core_h: usize = TILE_NUM_PER_CHUNK_DIM as usize;

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
            indices.extend_from_slice(&[v0, v3, v1, v0, v2, v3]);
        }
    }

    let dummy_normals = vec![[0.0, 1.0, 0.0]; estimated_vertex_count];
    let dummy_uv1s = vec![[0.0, 0.0]; estimated_vertex_count];

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, dummy_normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, dummy_uv1s);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// This startup system generates shared terrain meshes for multiple LODs.
pub fn setup_land_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let high = meshes.add(build_chunk_mesh(1)); // 8x8 quads, 9x9 vertices
    let medium = meshes.add(build_chunk_mesh(2)); // 4x4 quads, 5x5 vertices
    let low = meshes.add(build_chunk_mesh(4)); // 2x2 quads, 3x3 vertices

    commands.insert_resource(LandMeshHandles { high, medium, low });
    commands.insert_resource(LandMeshLod::default());
}
