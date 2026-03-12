use crate::{
    core::{render::scene::player::Player, system_sets::StartupSysSet},
    prelude::*,
};
use bevy::prelude::*;

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
    let font: Handle<Font> = asset_server.load("fonts/uo/UOClassicRough.ttf");

    let root_id = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            top: Val::Px(20.0),
            ..default()
        })
        .id();

    let bg_id = commands
        .spawn((
            Node {
                padding: UiRect::all(Val::Px(7.0)),
                ..default()
            },
            BackgroundColor(Color::BLACK.with_alpha(0.65)),
        ))
        .with_children(|builder| {
            builder.spawn((
                Text::new("Player position: (NA, NA, NA)"),
                TextFont {
                    font,
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                OverlayPlayerPositionText,
            ));
        })
        .id();

    commands.entity(root_id).add_child(bg_id);
}

pub fn update_player_position_text(
    player_query: Query<&Transform, With<Player>>,
    mut text_query: Query<&mut Text, With<OverlayPlayerPositionText>>,
) {
    if let (Ok(transform), Ok(mut text)) = (player_query.single(), text_query.single_mut()) {
        let pos = transform.translation.to_uo_vec3();
        *text = Text::new(format!(
            "Player position: [{}, {}, {}]",
            pos.x, pos.y, pos.z
        ));
    }
}
