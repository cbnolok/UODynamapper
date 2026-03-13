use crate::{
    core::{render::scene::player::Player, system_sets::StartupSysSet},
    prelude::*,
};
use bevy::prelude::*;
use bevy::text::LineHeight;

pub struct PlayerPositionOverlayPlugin;

impl Plugin for PlayerPositionOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            setup_overlay_player_position.in_set(StartupSysSet::SetupSceneStage2),
        )
        .add_systems(
            Update,
            update_player_position_text.run_if(in_state(AppState::InGame)),
        );
    }
}

#[derive(Component)]
pub struct OverlayPlayerPositionText;

pub fn setup_overlay_player_position(mut commands: Commands, asset_server: Res<AssetServer>) {
    println!("DEBUG: setup_overlay_player_position running");
    let font: Handle<Font> = asset_server.load("fonts/uo/UOClassicRough.ttf");

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(20.0),
                padding: UiRect::all(Val::Px(7.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            ZIndex(200),
        ))
        .with_children(|builder| {
            builder.spawn((
                Text::new("Player position: (NA, NA, NA)"),
                TextFont {
                    font,
                    font_size: 15.0,
                    ..default()
                },
                LineHeight::Px(15.0),
                TextColor(Color::WHITE),
                OverlayPlayerPositionText,
            ));
        });
}

pub fn update_player_position_text(
    player_query: Query<&Transform, With<Player>>,
    mut text_query: Query<&mut Text, With<OverlayPlayerPositionText>>,
) {
    if let (Some(transform), Some(mut text)) =
        (player_query.single().ok(), text_query.single_mut().ok())
    {
        let pos = transform.translation.to_uo_vec3();
        text.0 = format!("Player position: [{}, {}, {}]", pos.x, pos.y, pos.z);
    }
}
