use bevy::prelude::*;

pub fn setup_player_overlay(mut commands: Commands, asset_server: Res<AssetServer>) {
    // Root UI node
    commands.spawn(NodeBundle {
        style: Style {
            align_items: AlignItems::Start,
            justify_content: JustifyContent::Start,
            // Position it pinned to the top-left with some margin
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            top: Val::Px(20.0),
            ..default()
        },
        background_color: BackgroundColor(Color::NONE),
        ..default()
    })
    .with_children(|parent| {
        // Black rectangle background
        parent.spawn(NodeBundle {
            style: Style {
                padding: UiRect::all(Val::Px(7.0)),
                ..default()
            },
            background_color: BackgroundColor(Color::BLACK.with_a(0.8)),
            ..default()
        })
        .with_children(|parent| {
            // Player position text; use a marker for later update
            parent.spawn((
                TextBundle::from_section(
                    "Player: (0.00, 0.00, 0.00)",
                    TextStyle {
                        font: asset_server.load("fonts/FiraMono-Medium.ttf"),
                        font_size: 18.0,
                        color: Color::WHITE,
                    },
                ),
                PlayerPositionText,
            ));
        });
    });
}

// Marker so we can update the text
#[derive(Component)]
pub struct PlayerPositionText;

// System to update text
pub fn update_player_position_text(
    player_query: Query<&Transform, With<PlayerControlled>>,
    mut text_query: Query<&mut Text, With<PlayerPositionText>>,
) {
    if let (Ok(transform), Ok(mut text)) =
        (player_query.get_single(), text_query.get_single_mut())
    {
        let pos = transform.translation;
        text.sections[0].value =
            format!("Player: ({:.2}, {:.2}, {:.2})", pos.x, pos.y, pos.z);
    }
}
