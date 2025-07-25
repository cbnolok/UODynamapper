use crate::{
    core::{render::world::player::Player, system_sets::StartupSysSet},
    prelude::*,
};
use bevy::prelude::*;

pub struct OverlaysPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(OverlaysPlugin);

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            OnEnter(AppState::SetupSceneStage2),
            setup_overlay_player_position.in_set(StartupSysSet::SetupScene),
        )
        .add_systems(
            Update,
            update_player_position_text.run_if(in_state(AppState::InGame)),
        );
    }
}

// Marker so we can update the text
#[derive(Component)]
pub struct Overlay_PlayerPositionText;

pub fn setup_overlay_player_position(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font: Handle<Font> = asset_server.load("fonts/UOClassicRough.ttf"); // FiraMono-Medium

    // Camera (needed for UI)
    //commands.spawn(Camera2d);

    // Root UI node, pinned to the top left with margin
    let root_id = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            top: Val::Px(20.0),
            ..default()
        })
        .id();

    // Black rectangle background with padding for text
    let bg_id = commands
        .spawn((
            Node {
                padding: UiRect::all(Val::Px(7.0)),
                ..default()
            },
            BackgroundColor(Color::BLACK.with_alpha(0.7)),
        ))
        .with_children(|builder| {
            // Player position text, will be updated by system
            builder.spawn((
                Text::new("Player position: (NA, NA, NA)"),
                TextFont {
                    font,
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Overlay_PlayerPositionText,
            ));
        })
        .id();

    // Assemble node tree
    commands.entity(root_id).add_child(bg_id);
}

pub fn update_player_position_text(
    player_query: Query<&Transform, With<Player>>,
    mut text_query: Query<&mut Text, With<Overlay_PlayerPositionText>>,
) {
    if let (Ok(transform), Ok(mut text)) = (player_query.single(), text_query.single_mut()) {
        let pos = transform.translation;
        *text = Text::new(format!(
            "Player position: ({:.2}, {:.2}, {:.2})",
            pos.x, pos.y, pos.z
        ));
    }
}
