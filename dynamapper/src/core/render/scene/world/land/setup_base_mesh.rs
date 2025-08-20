use super::{TILE_NUM_PER_CHUNK_DIM, draw_mesh::LandMeshHandle};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::mesh::{Indices, PrimitiveTopology},
};

/// This startup system generates a single, shared 9x9 grid mesh for all land chunks.
pub fn setup_land_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    // Core: real tile number inside a map block.
    const CORE_W: usize = TILE_NUM_PER_CHUNK_DIM as usize;
    const CORE_H: usize = TILE_NUM_PER_CHUNK_DIM as usize;
    // The Grid we are making though has an additional row and column at south and east, so that it can contain the data about adjacent tiles.
    const GRID_W: usize = (TILE_NUM_PER_CHUNK_DIM + 1) as usize;
    const GRID_H: usize = (TILE_NUM_PER_CHUNK_DIM + 1) as usize;

    let estimated_vertex_count = GRID_W * GRID_H;
    let mut positions = Vec::with_capacity(estimated_vertex_count);
    let mut uvs = Vec::with_capacity(estimated_vertex_count);
    let mut indices = Vec::new();

    // Create a flat 9x9 grid of vertices at y=0
    // Add dummy height values (0.0) because the real one will be calculated on the gpu, via the shader
    //  (we send tile height through a uniform buffer).
    // We are adding an extra row and column to avoid seam artifacts and to make the neighboring chunk minimum tiles data
    //  available for the shader to calculate normals.
    for gy in 0..GRID_H {
        for gx in 0..GRID_W {
            positions.push([gx as f32, 0.0, gy as f32]);
            uvs.push([gx as f32 / (CORE_W as f32), gy as f32 / (CORE_H as f32)]);
        }
    }

    // Create indices for the 8x8 core of the grid
    for ty in 0..CORE_H {
        for tx in 0..CORE_W {
            let v0 = (ty * GRID_W + tx) as u32;
            let v1 = v0 + 1;
            let v2 = ((ty + 1) * GRID_W + tx) as u32;
            let v3 = v2 + 1;
            indices.extend_from_slice(&[v0, v3, v1, v0, v2, v3]);
        }
    }

    // Provide dummy normals and UV1s to match the shader's vertex format
    let dummy_normals = vec![[0.0, 1.0, 0.0]; estimated_vertex_count];
    let dummy_uv1s = vec![[0.0, 0.0]; estimated_vertex_count];

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, dummy_normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    // Add dummy UV1 values (a second copy of UV coords). We aren't really using them as those are, but by inserting them here
    //  we'll have them available in the render pipeline and we can use this variable to pass custom data from the vertex
    //  to the fragment shader.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, dummy_uv1s);
    mesh.insert_indices(Indices::U32(indices));

    let handle = meshes.add(mesh);
    commands.insert_resource(LandMeshHandle(handle));
}
