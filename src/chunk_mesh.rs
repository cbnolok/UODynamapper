use bevy::{
    prelude::*,
    pbr::ExtendedMaterial,
    render::{
        mesh::{Indices, PrimitiveTopology},
        render_asset::RenderAssetUsages,
    }
};
use bytemuck::Zeroable;

use crate::worldmap_base_mesh::*;
use crate::tile_cache::*;


pub struct UOMapTile { 
    art_id: u16, 
    hue: u16, 
    height: u16, // in UO it's i8 
}

// -- 0)  --------------------------------------------

const CHUNK_SIZE: i32 = 16;

#[derive(Component)]
pub struct MapMeshChunk {
    pub gx: i32,
    pub gy: i32,
}

// Fills your world with chunk meshes near the camera
pub fn build_visible_chunks(
    mut cmd:                Commands,
    mut meshes:             ResMut<Assets<Mesh>>,
    mut materials_terrain:  ResMut<Assets<ExtendedMaterial<StandardMaterial,TerrainMaterial>>>,
    //mut materials_std:     ResMut<Assets<StandardMaterial>>,  // sometimes used for debugging.
    mut cache:              ResMut<TileCache>,
    mut images:             ResMut<Assets<Image>>,
    cam_q:                  Query<&Transform, With<Camera3d>>,
    chunk_q:                Query<(Entity, &MapMeshChunk, Option<&Mesh3d>)>,
) {
    let cam_pos = cam_q.single().unwrap().translation;
    for (entity, chunk_data, mesh_handle) in chunk_q.iter() {
        if mesh_handle.is_some() {
            //println!("is_some = true!");
            continue;
        }
        let center = Vec3::new(
            (chunk_data.gx*CHUNK_SIZE + CHUNK_SIZE/2) as f32,
            0.0,
            (chunk_data.gy*CHUNK_SIZE + CHUNK_SIZE/2) as f32,
        );
        // spawn only if within 80 units
        if cam_pos.distance(center) > 80.0 {
            //println!("Too far!");
            continue;
        }

        // Build merged mesh for this chunk
        let mut verts = Vec::with_capacity((CHUNK_SIZE*CHUNK_SIZE*4) as usize);
        let mut idxs  = Vec::with_capacity((CHUNK_SIZE*CHUNK_SIZE*6) as usize);
        
        let mut terrain_uniforms = TerrainUniforms::zeroed();
        terrain_uniforms.light_dir = Vec3::Y;

        println!("Pre loop.");
        for ty in 0..CHUNK_SIZE {
            for tx in 0..CHUNK_SIZE {
                // ***** your real tile lookup goes here *****
                let tile = UOMapTile {
                    art_id: ((tx+ty)&0xFF) as u16,  // temp
                    hue:    0,
                    //heights: [0, ((tx+ty)&1) as i16, 0, 0],
                    height: ((tx+ty)&1) as u16 // temp,
                };
                let layer = cache.layer_of(tile.art_id, &mut cmd, &mut images);

                // corner positions and normals
                let x0 = (chunk_data.gx*CHUNK_SIZE + tx)     as f32;
                let x1 = x0 + 1.0;
                let z0 = (chunk_data.gy*CHUNK_SIZE + ty)     as f32;
                let z1 = z0 + 1.0;

                let h   = |i| tile.heights[i] as f32; // change this
                let n0 = calc_normal(h(0), h(1), h(3));
                let n1 = calc_normal(h(1), h(0), h(2));
                let n2 = calc_normal(h(2), h(1), h(3));
                let n3 = calc_normal(h(3), h(0), h(2));

                let base = verts.len() as u32;

                verts.push(TerrainVertexAttrs { pos: [x0,h(0),z0], uv: [0.,0.], norm: n0 });
                verts.push(TerrainVertexAttrs { pos: [x1,h(1),z0], uv: [1.,0.], norm: n1 });
                verts.push(TerrainVertexAttrs { pos: [x1,h(2),z1], uv: [1.,1.], norm: n2 });
                verts.push(TerrainVertexAttrs { pos: [x0,h(3),z1], uv: [0.,1.], norm: n3 });

                // Vertex winding order: counter-clockwise.
                idxs.extend([base, base + 2, base + 1, // 1st triangle
                    base, base + 3, base + 2]);             // 2nd triangle

                // Don't: clockwise means that the normal will face down, and we won't see the mesh.
                //idxs.extend([base, base+1, base+2, base, base+2, base+3]);

                /*
                println!(
                    "Creating geometry (vertices) for Chunk gx={},gy={} → verts={}, idxs={}",
                    chunk_data.gx, chunk_data.gy,
                    verts.len(), idxs.len()
                );
                */
            }
        }

        let chunk_mesh_handle: Handle<Mesh> = {
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD |  RenderAssetUsages::RENDER_WORLD);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts.iter().map(|v| v.pos).collect::<Vec<_>>());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL,   verts.iter().map(|v| v.norm).collect::<Vec<_>>());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0,     verts.iter().map(|v| v.uv).collect::<Vec<_>>());
            //mesh.insert_attribute(ATTR_LAYER_HUE,           verts.iter().map(|v| v.layer_hue).collect::<Vec<_>>());
            mesh.insert_indices(Indices::U32(idxs));
            meshes.add(mesh)
        };

        // Create the material, pointing at the texture array, with a fixed light dir
        let chunk_material_handle = {    
            let mat = ExtendedMaterial {
                base: StandardMaterial {
                    ..Default::default()
                },
                extension: TerrainMaterial {
                    tex_array: cache.image.clone(),
                    uniforms: terrain_uniforms,
                }
            };    
            materials_terrain.add(mat)
        };

        // Finally spawn the chunk’s visible mesh and its material.
        
        cmd.entity(entity)
        .insert((
            // Insert each of these as separate components on this entity.
            Mesh3d(chunk_mesh_handle.clone()),
            MeshMaterial3d(chunk_material_handle.clone()),
            // Other components go here, if needed.
        ));
    
        /*
        // For debugging
        cmd.spawn(PbrBundle {
            mesh:      mesh_handle.clone(),
            material:  _materials_std.add(StandardMaterial {
                base_color: Color::rgb(0.8,0.2,0.2),
                unlit:      true,    // so lighting doesn’t matter
                //cull_mode:  None,    // draw both sides
                ..default()
                }),
            transform: Transform::default(),
            ..default()
        });
        */

        println!("Spawned chunk at: gx={}, gy={}", chunk_data.gx, chunk_data.gy);
    }
}

// Normals via finite‐difference
fn calc_normal(h: f32, he: f32, hs: f32) -> [f32;3] {
    let dx = Vec3::new(1.0, he - h, 0.0);
    let dz = Vec3::new(0.0, hs - h, 1.0);
    dx.cross(dz).normalize().to_array()
}

