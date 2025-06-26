use bevy::prelude::*;

pub fn setup_cam(mut commands: Commands) {
    //------------------------------------
    // World light
    //------------------------------------
    
    // We won't use a world light source, we'll bake the light from the material and the shader.
    // We use it now just to light the "player" cube.
    /*
    // Set up a directional light (sun)
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false, // Disable shadows if not needed
            ..default()
        },
        Transform::from_xyz(8.0, 50.0, 8.0).looking_at(Vec3::new(8.0, 0.0, 8.0), Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the light in world space
    ));
    */

    //------------------------------------
    // Camera
    //------------------------------------

    // Center of the chunk/grid
    let center = Vec3::new(32.0, 0.0, 32.0);

    // Camera position: 30 units above & 30 units back
    let cam_pos = Vec3::new(center.x, 30.0, center.z + 30.0);

    commands.spawn((
        Camera3d::default(), // Marker component for 3D cameras
        Transform::from_xyz(cam_pos.x, cam_pos.y, cam_pos.z).looking_at(center, Vec3::Y),
        GlobalTransform::default(), // Needed for transforming the camera in world space
    ));
}

