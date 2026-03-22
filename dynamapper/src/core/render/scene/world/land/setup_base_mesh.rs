use super::draw_mesh::{ChunkScale, LandMeshHandles, LandMeshLod};
use bevy::{
    prelude::*,
    mesh::Indices,
    render::render_resource::PrimitiveTopology,
    asset::RenderAssetUsages,
};

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
            indices.extend_from_slice(&[v0, v3, v1, v0, v2, v3]);
        }
    }

    let dummy_normals = vec![[0.0, 1.0, 0.0]; estimated_vertex_count];
    let dummy_uv1s = vec![[0.0, 0.0]; estimated_vertex_count];

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
    // Standard 8×8 tile meshes at three vertex-density LODs:
    let high = meshes.add(build_chunk_mesh(8, 1));   // 81 verts  (zoom < 4)
    let medium = meshes.add(build_chunk_mesh(8, 2)); // 25 verts  (zoom 4–10)
    let low = meshes.add(build_chunk_mesh(8, 4));    //  9 verts  (zoom 10+ fallback)
    // Wide meshes for reduced entity count at high zoom:
    let wide16 = meshes.add(build_chunk_mesh(16, 2)); // 81 verts, covers 16×16 tiles (zoom 10–25)
    let wide32 = meshes.add(build_chunk_mesh(32, 4)); // 81 verts, covers 32×32 tiles (zoom 25–50)
    let wide64 = meshes.add(build_chunk_mesh(64, 8)); // 81 verts, covers 64×64 tiles (zoom ≥50)
    let wide128 = meshes.add(build_chunk_mesh(128, 16)); // 81 verts, covers 128×128 tiles (extreme zoom-out)
    let wide256 = meshes.add(build_chunk_mesh(256, 32)); // 81 verts, covers 256×256 tiles (maximum zoom-out)

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
