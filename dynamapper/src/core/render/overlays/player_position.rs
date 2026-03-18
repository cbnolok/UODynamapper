use crate::{
    core::render::scene::player::Player,
    prelude::*,
};
use bevy::prelude::*;
use bevy::text::{FontSmoothing, LineHeight};
use bevy::window::Window;
use crate::core::render::scene::camera::RenderZoom;
use crate::core::render::scene::camera::PlayerCamera;

pub struct PlayerPositionOverlayPlugin;

impl Plugin for PlayerPositionOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), setup_overlay_player_position)
            .add_systems(
                Update,
                update_player_position_text.run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Component)]
pub struct OverlayPlayerPositionContainer;

#[derive(Component)]
pub struct OverlayPlayerPositionText;

pub fn setup_overlay_player_position(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<crate::external_data::settings::Settings>,
) {
    let font: Handle<Font> = asset_server.load("fonts/uo/UOClassicRough.ttf");

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(20.0),
                padding: UiRect::all(Val::Px(7.0 * settings.app.window.overlay_scale)),
                display: if settings.app.performance.show_overlay {
                    Display::Flex
                } else {
                    Display::None
                },
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            GlobalZIndex(100),
            OverlayPlayerPositionContainer,
        ))
        .with_children(|builder| {
            let scale = settings.app.window.overlay_scale;
            builder.spawn((
                Text::new("Player position: (NA, NA, NA)"),
                TextFont {
                    font,
                    font_size: 15.0 * scale,
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..default()
                },
                LineHeight::Px(15.0 * scale),
                TextColor(Color::WHITE),
                OverlayPlayerPositionText,
            ));
        });
}

pub fn update_player_position_text(
    settings: Res<crate::external_data::settings::Settings>,
    player_query: Query<&Transform, With<Player>>,
    mut text_query: Query<
        (&mut Text, &mut TextFont, &mut LineHeight),
        With<OverlayPlayerPositionText>,
    >,
    mut node_query: Query<
        &mut Node,
        (
            With<OverlayPlayerPositionContainer>,
            Without<OverlayPlayerPositionText>,
        ),
    >,
    mut last_scale: Local<f32>,
    windows: Query<&Window>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    render_zoom: Res<RenderZoom>,
) {
    let current_scale = settings.app.window.overlay_scale;
    let scale_changed = (*last_scale - current_scale).abs() > 0.001;

    // Real-time visibility toggle from settings
    if let Ok(mut node) = node_query.single_mut() {
        let target_display = if settings.app.performance.show_overlay {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != target_display {
            node.display = target_display;
        }
    }

    if let (Some(transform), Some((mut text, mut text_font, mut line_height))) =
        (player_query.single().ok(), text_query.single_mut().ok())
    {
        let pos = transform.translation.to_uo_vec3();
        // Compute viewport tile coverage by sampling the camera frustum
        let mut viewport_tiles = "-- x --".to_string();
        if let (Ok(window), Ok((cam, cam_tf))) = (windows.single(), camera_q.single()) {
            let window_w = window.width();
            let window_h = window.height();
            let sample_points = [
                Vec2::new(0.0, 0.0),
                Vec2::new(window_w, 0.0),
                Vec2::new(window_w, window_h),
                Vec2::new(0.0, window_h),
                Vec2::new(window_w * 0.5, 0.0),
                Vec2::new(window_w, window_h * 0.5),
                Vec2::new(window_w * 0.5, window_h),
                Vec2::new(0.0, window_h * 0.5),
            ];

            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut min_z = f32::INFINITY;
            let mut max_z = f32::NEG_INFINITY;
            let mut any_hit = false;

            for &screen_pt in &sample_points {
                if let Ok(ray) = cam.viewport_to_world(cam_tf, screen_pt) {
                    let dir_y = ray.direction.y;
                    if dir_y.abs() > 1e-6 {
                        let t = -ray.origin.y / dir_y;
                        let hit = ray.origin + *ray.direction * t;
                        min_x = min_x.min(hit.x);
                        max_x = max_x.max(hit.x);
                        min_z = min_z.min(hit.z);
                        max_z = max_z.max(hit.z);
                        any_hit = true;
                    }
                }
            }

            if any_hit {
                let tiles_w = ((max_x - min_x).abs().ceil()) as i32;
                let tiles_h = ((max_z - min_z).abs().ceil()) as i32;
                viewport_tiles = format!("{} x {}", tiles_w, tiles_h);
            }
        }

        text.0 = format!(
            "Player position: [{}, {}, {}]\nRender zoom: {:.2}\nViewport: {} tiles",
            pos.x, pos.y, pos.z, render_zoom.0, viewport_tiles
        );

        if scale_changed {
            text_font.font_size = 15.0 * current_scale;
            *line_height = LineHeight::Px(15.0 * current_scale);
            *last_scale = current_scale;
        }
    }
}
