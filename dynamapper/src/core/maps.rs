use bevy::ecs::resource::Resource;

#[derive(Resource, Default)]
pub struct MapPlane {
    pub id: u8,
    pub width: u32,
    pub height: u32,
}
