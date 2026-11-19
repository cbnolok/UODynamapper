use super::*;

fn sample(height: f32) -> Option<TerrainSample> { Some(TerrainSample { height, ec_present: false, liquid: false }) }
fn grid(step: u32) -> Grid { Grid { origin: UVec2::ZERO, span: 4, step } }
fn down(x: f32, y: f32) -> Ray3d { Ray3d::new(Vec3::new(x, 20.0, y), Dir3::NEG_Y) }
fn close(actual: Vec3, expected: Vec3) { assert!((actual - expected).abs().max_element() < 0.0001, "{actual:?} != {expected:?}"); }

#[test]
fn terrain_cursor_uses_triangle_diagonal_and_interpolated_surface_not_zero_plane() {
    let read = |cell: IVec2| sample(if cell == IVec2::ONE { 4.0 } else { 0.0 });
    close(trace_grids(down(0.75, 0.25), Vec2::ZERO, &[grid(1)], &read).unwrap(), Vec3::new(0.75, 1.0, 0.25));
    close(trace_grids(down(0.25, 0.75), Vec2::ZERO, &[grid(1)], &read).unwrap(), Vec3::new(0.25, 1.0, 0.75));
    // An oblique ray meets the raised triangle before its Y=0 intersection.
    let target = Vec3::new(0.75, 1.0, 0.25);
    let ray = Ray3d::new(target + Vec3::new(1.0, 8.0, 1.0), Dir3::new(Vec3::new(-1.0, -8.0, -1.0)).unwrap());
    // Restrict this fixture to one quad; the adjacent descending face would
    // correctly occlude the target when the whole peak is admitted.
    close(trace_grids(ray, Vec2::ZERO, &[Grid { span: 1, ..grid(1) }], &read).unwrap(), target);
}

#[test]
fn terrain_cursor_uses_assigned_coarse_mesh_and_current_admitted_chunks_only() {
    let read = |cell: IVec2| sample(if cell == IVec2::ONE { 8.0 } else { 0.0 });
    close(trace_grids(down(1.0, 1.0), Vec2::ZERO, &[grid(1)], &read).unwrap(), Vec3::new(1.0, 8.0, 1.0));
    close(trace_grids(down(1.0, 1.0), Vec2::ZERO, &[grid(2)], &read).unwrap(), Vec3::new(1.0, 0.0, 1.0));
    assert!(trace_grids(down(1.0, 1.0), Vec2::ZERO, &[], &read).is_none());
    assert!(trace_grids(down(6.0, 1.0), Vec2::ZERO, &[grid(1)], &read).is_none());
    assert!(trace_grids(down(1.0, 1.0), Vec2::ZERO, &[grid(1)], &|_| None).is_none());
}

#[test]
fn terrain_cursor_rejects_behind_ray_and_backfaces_but_accepts_horizontal_slopes() {
    let read = |cell: IVec2| sample(cell.x as f32);
    let ray = Ray3d::new(Vec3::new(-1.0, 2.0, 0.5), Dir3::X);
    close(trace_grids(ray, Vec2::ZERO, &[grid(1)], &read).unwrap(), Vec3::new(2.0, 2.0, 0.5));
    assert!(trace_grids(Ray3d::new(Vec3::new(1.0, 20.0, 1.0), Dir3::Y), Vec2::ZERO, &[grid(1)], &read).is_none());
    assert!(trace_grids(Ray3d::new(Vec3::new(1.0, -20.0, 1.0), Dir3::Y), Vec2::ZERO, &[grid(1)], &read).is_none());
}

#[test]
fn terrain_cursor_retains_signed_z_extremes_and_facet_sized_coordinates() {
    let grid = Grid { origin: UVec2::new(7000, 4000), span: 4, step: 1 };
    for raw_z in [i8::MIN, -5, 0, 30, i8::MAX] {
        let expected = Vec3::new(7001.25, f32::from(raw_z) * UO_Z_TO_WORLD_SCALE, 4002.75);
        close(trace_grids(down(expected.x, expected.z), Vec2::ZERO, &[grid], &|_| sample(expected.y)).unwrap(), expected);
    }
}

#[test]
fn terrain_cursor_selects_nearest_visible_hill_independent_of_chunk_order() {
    let near = Grid { origin: UVec2::new(4, 4), ..grid(1) };
    let far = grid(1);
    let ray = Ray3d::new(Vec3::new(10.0, 10.0, 10.0), Dir3::new(-Vec3::ONE).unwrap());
    let read = |cell: IVec2| sample(if cell.x >= 4 && cell.y >= 4 { 6.0 } else { 0.0 });
    for grids in [[near, far], [far, near]] {
        close(trace_grids(ray, Vec2::ZERO, &grids, &read).unwrap(), Vec3::splat(6.0));
    }
}

#[test]
fn terrain_cursor_bank_relaxation_is_view_facing_and_keeps_liquid_level() {
    let read = |cell: IVec2| Some(TerrainSample { height: if cell == IVec2::ONE { 5.0 } else { 3.0 },
        ec_present: true, liquid: cell != IVec2::ONE });
    close(bank_vertex(IVec2::ZERO, Vec2::ONE, &read).unwrap(), Vec3::new(0.25, 3.0, 0.25));
    close(bank_vertex(IVec2::ZERO, -Vec2::ONE, &read).unwrap(), Vec3::new(0.0, 3.0, 0.0));
    close(bank_vertex(IVec2::ONE, Vec2::ONE, &read).unwrap(), Vec3::new(1.0, 4.85, 1.0));
    // CC and missing EC neighbors never trigger bank geometry.
    let missing = |cell: IVec2| read(cell).map(|mut s| { s.ec_present = false; s });
    close(bank_vertex(IVec2::ONE, Vec2::ONE, &missing).unwrap(), Vec3::new(1.0, 5.0, 1.0));
}

#[test]
fn terrain_cursor_mesh_grids_match_all_generated_mesh_dimensions() {
    use bevy::mesh::VertexAttributeValues;
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>().add_systems(Startup, super::super::setup_base_mesh::setup_land_mesh);
    app.update();
    let handles = app.world().resource::<LandMeshHandles>();
    let meshes = app.world().resource::<Assets<Mesh>>();
    for handle in [&handles.high, &handles.medium, &handles.low, &handles.wide16, &handles.wide32,
        &handles.wide64, &handles.wide128, &handles.wide256] {
        let (span, step) = handles.grid(&Mesh3d(handle.clone())).unwrap();
        let mesh = meshes.get(handle).unwrap();
        let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else { panic!("positions") };
        assert_eq!(positions.len(), ((span / step + 1).pow(2)) as usize);
        assert_eq!(positions.last().copied(), Some([span as f32, 0.0, span as f32]));
    }
}

#[test]
fn terrain_cursor_ray_matches_physical_pixels_in_both_projections_and_inset_hidpi_viewports() {
    use bevy::camera::{ComputedCameraValues, RenderTargetInfo, Viewport};
    for perspective in [false, true] { for scale in [1.0, 2.0] {
        let mut projection = if perspective { Projection::Perspective(PerspectiveProjection::default()) }
            else { Projection::Orthographic(OrthographicProjection { scaling_mode: bevy::camera::ScalingMode::Fixed { width: 4.0, height: 4.0 }, ..OrthographicProjection::default_3d() }) };
        projection.update(1200.0, 760.0);
        let camera = Camera { viewport: Some(Viewport { physical_position: (Vec2::new(40.0, 20.0) * scale).as_uvec2(),
            physical_size: (Vec2::new(1200.0, 760.0) * scale).as_uvec2(), ..default() }),
            computed: ComputedCameraValues { clip_from_view: projection.get_clip_from_view(),
                target_info: Some(RenderTargetInfo { physical_size: (Vec2::new(1280.0, 800.0) * scale).as_uvec2(), scale_factor: scale }),
                ..default() }, ..default() };
        let transform = GlobalTransform::from(Transform::from_xyz(2.0, 10.0, 2.0).looking_at(Vec3::new(2.0, 0.0, 2.0), Vec3::Z));
        let pointer = Vec2::new(650.1, 390.7);
        let ray = cursor_ray(&camera, &transform, pointer).unwrap();
        let hit = trace_grids(ray, Vec2::ZERO, &[grid(1)], &|_| sample(0.0)).unwrap();
        let projected = camera.world_to_viewport(&transform, hit).unwrap();
        let expected = ((pointer * scale).floor() + Vec2::splat(0.5)) / scale;
        assert!((projected - expected).abs().max_element() < 0.002, "{projected:?} != {expected:?}");
        assert!(cursor_ray(&camera, &transform, Vec2::splat(5.0)).is_none());
    }}
}
