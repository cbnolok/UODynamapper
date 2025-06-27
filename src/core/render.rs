pub mod camera;
pub mod player;
pub mod scene;

use bevy::prelude::*;

pub struct RenderPlugin;
impl Plugin for RenderPlugin
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

