//! The camera's controls: wheel and +/- zoom, and the free camera's keys. The camera plugin registers them, in its own
//! order; what they move (the zoom, the camera's transform) belongs to `render::view::camera`.
use crate::render::scene::RecomputeVisibleChunksEvent;
use crate::render::view::camera::{perspective, PlayerCamera, RenderZoom};
use crate::settings::Settings;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::Window;
use bevy_egui::EguiContexts;

/// Each zoom step multiplies or divides the zoom by this: perceptually even steps, about 25 of them from 1x to 50x.
const ZOOM_STEP_FACTOR: f32 = 1.15;
/// The free camera's pan and height speed, in world units per second.
const FREE_CAMERA_MOVE_SPEED: f32 = 10.0;
/// The free camera's rotation speed, in radians per second (about 8.6 degrees).
const FREE_CAMERA_ROTATE_SPEED: f32 = 0.15;

/// Zooms on the mouse wheel and the +/- keys, unless egui has the pointer or the keyboard. A free perspective camera
/// dollies along its view instead of scaling, and every step asks the streaming for a recompute.
pub(crate) fn sys_camera_zoom(
    mut zoom_res: ResMut<RenderZoom>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut kbd_events: MessageReader<KeyboardInput>,
    mut egui_contexts: EguiContexts,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    settings: Res<Settings>,
    mut camera_q: Query<&mut Transform, With<PlayerCamera>>,
    windows: Query<&Window>,
) {
    // If egui is using the mouse/keyboard, don't zoom
    let ctx = match egui_contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };
    if ctx.egui_wants_pointer_input() || ctx.egui_wants_keyboard_input() {
        return;
    }

    let mut zoom_delta = 0.0;

    // Mouse scroll
    for event in scroll_events.read() {
        zoom_delta -= event.y; // Accumulate raw scroll ticks (typically ±1)
    }

    // Keyboard +/-
    for ev in kbd_events.read() {
        if ev.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        if let Key::Character(input) = &ev.logical_key {
            match input.as_str() {
                "+" | "=" => zoom_delta -= 1.0,
                "-" | "_" => zoom_delta += 1.0,
                _ => {}
            }
        }
    }

    if zoom_delta != 0.0 {
        // Exponential zoom: each scroll tick multiplies/divides by a constant factor.
        // This gives perceptually uniform zoom steps — big jumps when zoomed out,
        // fine control when zoomed in. ~25 ticks to go from 1× to 50×.
        let multiplier = ZOOM_STEP_FACTOR.powf(zoom_delta);
        let previous_zoom = zoom_res.0;
        let new_zoom = previous_zoom * multiplier;
        zoom_res.write_val(new_zoom);
        if settings.app.window.free_camera && settings.app.window.perspective_camera {
            if let (Ok(mut transform), Ok(window)) = (camera_q.single_mut(), windows.single()) {
                // Follow perspective applies zoom to its orbit distance. Free
                // perspective uses the same dolly distance along its current
                // viewing direction, preserving any manual pan and rotation.
                let window_height = window.resolution.height();
                let distance = perspective::orbit_distance(window_height, zoom_res.0)
                    - perspective::orbit_distance(window_height, previous_zoom);
                let backward = *transform.back();
                transform.translation += backward * distance;
            }
        }
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
    }
}

/// Moves the free camera: the arrows pan, Shift+Up/Down change its height, and Right Shift with W/S, A/D and Q/E
/// pitch, yaw and roll it. Ignored while egui has the keyboard.
pub(crate) fn sys_free_camera_movement(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut camera_q: Query<&mut Transform, With<PlayerCamera>>,
    mut egui_contexts: EguiContexts,
) {
    let ctx = match egui_contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };
    if ctx.egui_wants_keyboard_input() {
        return;
    }

    let Some(mut transform) = camera_q.single_mut().ok() else {
        return;
    };

    let move_speed = FREE_CAMERA_MOVE_SPEED * time.delta_secs();
    let rotate_speed = FREE_CAMERA_ROTATE_SPEED * time.delta_secs();
    let mut move_vec = Vec3::ZERO;

    if keyboard.pressed(KeyCode::ArrowUp) {
        move_vec.z -= 1.0;
        move_vec.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        move_vec.z += 1.0;
        move_vec.x += 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowLeft) {
        move_vec.x -= 1.0;
        move_vec.z += 1.0;
    }
    if keyboard.pressed(KeyCode::ArrowRight) {
        move_vec.x += 1.0;
        move_vec.z -= 1.0;
    }

    // Altitude
    let mut y_delta = 0.0;
    if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
        if keyboard.pressed(KeyCode::ArrowUp) {
            y_delta += 1.0;
        }
        if keyboard.pressed(KeyCode::ArrowDown) {
            y_delta -= 1.0;
        }
    }

    // Rotation (Yaw, Pitch, Roll) - Only active when Right Shift is held
    if keyboard.pressed(KeyCode::ShiftRight) {
        // Yaw (A/D)
        if keyboard.pressed(KeyCode::KeyA) {
            transform.rotate_local_y(rotate_speed);
        }
        if keyboard.pressed(KeyCode::KeyD) {
            transform.rotate_local_y(-rotate_speed);
        }
        // Pitch (W/S)
        if keyboard.pressed(KeyCode::KeyW) {
            transform.rotate_local_x(rotate_speed);
        }
        if keyboard.pressed(KeyCode::KeyS) {
            transform.rotate_local_x(-rotate_speed);
        }
        // Roll (Q/E)
        if keyboard.pressed(KeyCode::KeyQ) {
            transform.rotate_local_z(rotate_speed);
        }
        if keyboard.pressed(KeyCode::KeyE) {
            transform.rotate_local_z(-rotate_speed);
        }
    }

    if move_vec != Vec3::ZERO {
        transform.translation += move_vec.normalize() * move_speed;
    }

    if y_delta != 0.0 {
        transform.translation.y += y_delta * move_speed;
    }
}
