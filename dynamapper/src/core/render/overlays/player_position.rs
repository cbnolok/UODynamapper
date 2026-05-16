use crate::core::render::scene::camera::RenderZoom;
use crate::{core::render::scene::player::Player, prelude::*};
use bevy::prelude::*;
use bevy::text::{FontSmoothing, LineHeight};
use bevy::window::Window;

const FONT_SIZE: f32 = 18.0;

use bevy::time::common_conditions::on_real_timer;
use std::time::Duration;

pub struct PlayerPositionOverlayPlugin {
    pub registered_by: &'static str,
}
crate::impl_tracked_plugin!(PlayerPositionOverlayPlugin);

impl Plugin for PlayerPositionOverlayPlugin {
    fn build(&self, app: &mut App) {
        crate::util_lib::tracked_plugin::log_plugin_build(self);
        app.add_systems(OnEnter(AppState::InGame), setup_overlay_player_position)
            .add_systems(
                Update,
                update_player_position_text
                    .run_if(in_state(AppState::InGame))
                    .run_if(on_real_timer(Duration::from_secs_f32(1.0 / 8.0))),
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
    settings: Res<crate::configs::settings::Settings>,
) {
    crate::util_lib::tracked_plugin::log_system_add_one_shot::<PlayerPositionOverlayPlugin>(
        "OnEnter(InGame)",
        "None",
        crate::fname!(),
    );
    let font: Handle<Font> = asset_server.load("fonts/uo/UOClassicRough.ttf");

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(20.0),
                padding: UiRect::all(Val::Px(7.0 * settings.app.window.player_position_scale)),
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
            let scale = settings.app.window.player_position_scale;
            builder.spawn((
                Text::new("Player position: (NA, NA, NA)"),
                TextFont {
                    font: font.clone(),
                    font_size: FONT_SIZE * scale,
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..default()
                },
                LineHeight::Px(FONT_SIZE * scale),
                TextColor(Color::WHITE),
                OverlayPlayerPositionText,
            ));
        });
}

pub fn update_player_position_text(
    settings: Res<crate::configs::settings::Settings>,
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
    mut last_player_pos: Local<Option<Vec3>>,
    mut last_window_size: Local<Option<(f32, f32)>>,
    mut last_render_zoom: Local<f32>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
) {
    let show_overlay = settings.app.performance.show_overlay;

    // 1. Real-time visibility toggle from settings
    if let Ok(mut node) = node_query.single_mut() {
        let target_display = if show_overlay {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != target_display {
            node.display = target_display;
        }
    }

    // 2. Early exit if not showing
    if !show_overlay {
        return;
    }

    let current_scale = settings.app.window.player_position_scale;
    let scale_changed = (*last_scale - current_scale).abs() > 0.001;

    if let (Some(transform), Ok((mut text, mut text_font, mut line_height))) =
        (player_query.iter().next(), text_query.single_mut())
    {
        let pos = transform.translation;

        let window_size = windows
            .single()
            .ok()
            .map(|window| (window.width(), window.height()));

        let needs_rebuild = *last_player_pos != Some(pos)
            || *last_window_size != window_size
            || (render_zoom.0 - *last_render_zoom).abs() > f32::EPSILON;

        if needs_rebuild {
            use std::fmt::Write;
            let mut buffer = String::with_capacity(256);

            let pos_uo = pos.to_uo_vec3();
            let zoom = render_zoom.0;

            let mut tiles_w = 0;
            let mut tiles_h = 0;
            let mut has_viewport = false;

            if let Some((window_w, window_h)) = window_size {
                // Optimization: Use direct isometric math instead of 8 raycasts.
                // Logic mirrored from scene.rs :: compute_visible_chunks.
                use crate::core::render::scene::camera::{ORTHO_SIZE_FACTOR, ORTHO_WIDTH_SCALE_FACTOR};
                let inv_sqrt2: f32 = std::f32::consts::FRAC_1_SQRT_2;
                let three_over_sqrt6: f32 = 3.0 / 6.0_f32.sqrt();

                let ortho_width = window_w / ORTHO_SIZE_FACTOR;
                let ortho_height = (window_h / ORTHO_WIDTH_SCALE_FACTOR) / ORTHO_SIZE_FACTOR;

                let hw = ortho_width * zoom / 2.0;
                let hh = ortho_height * zoom / 2.0;

                // Span calculation: how many world units are visible across the screen.
                // In iso view, the span is a combination of horizontal (u) and vertical (v) frustum extents.
                tiles_w = (2.0 * hw * inv_sqrt2).ceil() as i32;
                tiles_h = (2.0 * hh * three_over_sqrt6).ceil() as i32;
                has_viewport = true;
            }

            let _ = write!(
                &mut buffer,
                "Player position: [{}, {}, {}]\nRender zoom: {:.2}\nViewport: ",
                pos_uo.x, pos_uo.y, pos_uo.z, zoom
            );

            if has_viewport {
                let _ = write!(&mut buffer, "{} x {} tiles", tiles_w, tiles_h);
            } else {
                let _ = write!(&mut buffer, "-- x -- tiles");
            }

            if text.0 != buffer {
                text.0 = buffer;
            }

            *last_player_pos = Some(pos);
            *last_window_size = window_size;
            *last_render_zoom = zoom;
        }

        if scale_changed {
            text_font.font_size = FONT_SIZE * current_scale;
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
            *last_scale = current_scale;
        }
    }
}
