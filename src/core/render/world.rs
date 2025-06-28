pub mod camera;
pub mod player;
pub mod scene;

use bevy::prelude::*;

pub struct WorldPlugin;
impl Plugin for WorldPlugin
{
    fn build(&self, app: &mut App) {
        app
            .add_plugins((
                camera::CameraPlugin,
                player::PlayerPlugin,
                scene::ScenePlugin,
            ));
    }
}

