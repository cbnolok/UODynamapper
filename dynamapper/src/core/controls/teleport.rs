use crate::core::render::scene::{player::Player, world::WorldGeoData};
use crate::prelude::UOVec4;
use bevy::prelude::Transform;

/// Shared by the dialog and development control channel. Callers request a
/// visible-chunk recompute only after this validated state change succeeds.
pub(crate) fn teleport_player(
    player: &mut Player,
    transform: &mut Transform,
    target: UOVec4,
    world: &WorldGeoData,
) -> Result<(), String> {
    let map = world.maps.get(&u32::from(target.m))
        .ok_or_else(|| format!("Map {} is not loaded", target.m))?;
    if u32::from(target.x) >= map.width || u32::from(target.y) >= map.height {
        return Err(format!("Target [{}, {}] is outside map {} ({}x{})", target.x, target.y, target.m, map.width, map.height));
    }
    player.current_pos = Some(target);
    transform.translation = target.to_bevy_vec3_ignore_map();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::maps::MapPlaneMetadata;

    #[test]
    fn teleport_validates_before_changing_player_or_transform() {
        let mut world = WorldGeoData::default();
        world.maps.insert(0, MapPlaneMetadata { id: 0, width: 7168, height: 4096 });
        let start = UOVec4::new(2834, 987, 0, 0);
        let mut player = Player { current_pos: Some(start), prev_rendered_pos: Some(start) };
        let mut transform = Transform::from_translation(start.to_bevy_vec3_ignore_map());
        for invalid in [UOVec4::new(7168, 561, 0, 0), UOVec4::new(2518, 4096, 0, 0), UOVec4::new(2518, 561, 0, 1)] {
            assert!(teleport_player(&mut player, &mut transform, invalid, &world).is_err());
            assert!(player.current_pos == Some(start));
            assert_eq!(transform.translation, start.to_bevy_vec3_ignore_map());
        }
        let target = UOVec4::new(2518, 561, -5, 0);
        teleport_player(&mut player, &mut transform, target, &world).unwrap();
        assert!(player.current_pos == Some(target));
        assert_eq!(transform.translation, target.to_bevy_vec3_ignore_map());
        assert!(player.prev_rendered_pos == Some(start));
    }
}
