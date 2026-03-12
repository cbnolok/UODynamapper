// Options dialog (egui window)
//
// Keybinding: F2 = toggle this dialog open/closed.
//
// Currently contains:
//   - Frame Limiter: checkbox to enable/disable + combobox to pick target FPS.
//     Writes to `bevy_framepace::FramepaceSettings` directly.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use bevy_framepace::{FramepaceSettings, Limiter};
use crate::prelude::*;

// Keybinding to toggle the options dialog.
const KEY_TOGGLE_OPTIONS: KeyCode = KeyCode::F2;

/// Preset FPS values offered in the combobox.
/// Using a fixed list keeps the UI simple and avoids having a free-form number input
/// that could produce unrealistic or unstable values.
const FPS_PRESETS: &[u32] = &[30, 60, 75, 120, 144, 165, 240];

/// Default FPS cap used when the user first enables frame limiting.
const DEFAULT_FPS_LIMIT: u32 = 60;

// ---- State resource ----

/// Holds the UI state for the options dialog.
/// All fields are the "UI representation" of the settings; actual values are applied
/// to their respective Bevy resources on change.
#[derive(Resource)]
pub struct OptionsDialogState {
    /// Whether the dialog window is open.
    pub open: bool,
    /// Whether frame limiting is active.
    pub frame_limit_enabled: bool,
    /// The target FPS when frame limiting is enabled.
    pub fps_preset_idx: usize,
    /// Speed multiplier for player movement.
    pub movement_speed_multiplier: f32,
    /// Whether to hide the player mesh.
    pub hide_player: bool,
    /// Whether to show the performance overlay.
    pub show_overlay: bool,
}

impl Default for OptionsDialogState {
    fn default() -> Self {
        let fps_preset_idx = FPS_PRESETS
            .iter()
            .position(|&fps| fps == DEFAULT_FPS_LIMIT)
            .unwrap_or(0);
        Self {
            open: false,
            frame_limit_enabled: true,
            fps_preset_idx,
            movement_speed_multiplier: 1.0,
            hide_player: false,
            show_overlay: true,
        }
    }
}

// ---- Plugin ----

pub struct OptionsDialogPlugin;

impl Plugin for OptionsDialogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OptionsDialogState>()
            .add_systems(Update, (
                sys_toggle_options_dialog.run_if(in_state(AppState::InGame)),
                sys_sync_settings_to_state.run_if(in_state(AppState::InGame))
            ))
            .add_systems(EguiPrimaryContextPass, sys_render_options_dialog.run_if(in_state(AppState::InGame)));
    }
}

/// One-time sync from Settings resource to UI state when dialog is first initialized or loaded.
fn sys_sync_settings_to_state(
    settings: Res<Settings>,
    mut state: ResMut<OptionsDialogState>,
    mut initialized: Local<bool>,
) {
    if !*initialized {
        state.movement_speed_multiplier = settings.app.input.movement_speed_multiplier;
        state.hide_player = settings.core.world.hide_player;
        state.show_overlay = settings.app.performance.show_overlay;
        state.frame_limit_enabled = settings.app.performance.frame_limit_enabled;
        state.fps_preset_idx = FPS_PRESETS
            .iter()
            .position(|&fps| fps == settings.app.performance.target_fps)
            .unwrap_or(0);
        *initialized = true;
    }
}

// ---- Systems ----

/// Toggles the options dialog on F2; also allows Escape to close it.
fn sys_toggle_options_dialog(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<OptionsDialogState>,
) {
    if keyboard.just_pressed(KEY_TOGGLE_OPTIONS) {
        state.open = !state.open;
    }
    // Pressing Escape closes the dialog if it is open.
    if keyboard.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
    }
}

/// Renders the options dialog window and applies any changes to the relevant resources.
fn sys_render_options_dialog(
    mut contexts: EguiContexts,
    mut state: ResMut<OptionsDialogState>,
    mut framepace: ResMut<FramepaceSettings>,
    mut settings: ResMut<Settings>,
) {
    let ctx = match contexts.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };

    // — Borrow fix —
    // `egui::Window::open()` mutably borrows the bool for the lifetime of the show()
    // call, which clashes with re-borrowing `state` inside the closure.
    // Solution: stage the value in a local, pass a reference to that local, then
    // write the (possibly changed by egui's own close button) value back afterward.
    let mut window_open = state.open;

    let response = egui::Window::new("Options [F2]")
        .default_pos([200.0, 80.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            ui.heading("Performance");
            ui.separator();

            // ---- Frame limiter toggle ----
            // The checkbox enables or disables the bevy_framepace limiter.
            // When disabled we set Limiter::Off so the GPU renders as fast as it can.
            let prev_enabled = state.frame_limit_enabled;
            ui.checkbox(&mut state.frame_limit_enabled, "Enable frame limiter");

            // ---- FPS combobox (greyed out when frame limiting is off) ----
            // `add_enabled` renders controls in a visually disabled state when the
            // first argument is `false`, preventing interaction without extra logic.
            ui.add_enabled_ui(state.frame_limit_enabled, |ui| {
                let current_fps = FPS_PRESETS[state.fps_preset_idx];
                egui::ComboBox::from_label("Target FPS")
                    .selected_text(format!("{} fps", current_fps))
                    .show_ui(ui, |ui| {
                        for (idx, &fps) in FPS_PRESETS.iter().enumerate() {
                            let label = format!("{} fps", fps);
                            // `selectable_value` automatically updates `state.fps_preset_idx`
                            // and returns whether the selection changed.
                            ui.selectable_value(&mut state.fps_preset_idx, idx, label);
                        }
                    });
            });

            // ---- Apply changes to FramepaceSettings ----
            // We apply whenever either the toggle or the combobox selection changed.
            // Comparing against the previous state avoids writing every frame.
            let fps_changed = state.fps_preset_idx != {
                // Derive what the previous index would have been from the current limiter.
                // If we can't determine it (e.g. limiter was Off), we just use usize::MAX
                // so the comparison always triggers a write on first open — harmless.
                match framepace.limiter {
                    Limiter::Manual(d) => {
                        let current_fps_hz = 1.0 / d.as_secs_f64();
                        FPS_PRESETS
                            .iter()
                            .position(|&fps| (fps as f64 - current_fps_hz).abs() < 0.5)
                            .unwrap_or(usize::MAX)
                    }
                    _ => usize::MAX,
                }
            };

            if prev_enabled != state.frame_limit_enabled || fps_changed {
                framepace.limiter = if state.frame_limit_enabled {
                    // `Limiter::from_framerate` converts the FPS value to a `Duration` and
                    // returns `Limiter::Manual(duration)`.
                    Limiter::from_framerate(FPS_PRESETS[state.fps_preset_idx] as f64)
                } else {
                    // `Limiter::Off` disables sleep entirely; bevy_framepace stays loaded
                    // (for frame pacing) but applies no artificial limit.
                    Limiter::Off
                };
            }

            ui.label(
                egui::RichText::new(
                    "Note: frame limiting also affected by VSync (driver/compositor level).",
                )
                .small()
                .weak(),
            );

            ui.add_space(8.0);
            ui.heading("World & Input");
            ui.separator();

            // ---- Movement Speed ----
            ui.horizontal(|ui| {
                ui.label("Move Speed:");
                ui.add(egui::Slider::new(&mut state.movement_speed_multiplier, 0.1..=40.0));
            });

            // ---- Visibility/Graphics ----
            ui.checkbox(&mut state.hide_player, "Hide Player Object");
            ui.checkbox(&mut state.show_overlay, "Show Performance Overlay");

            // ---- Sync UI state to Settings resource ----
            // We use ResMut here every frame we detect a difference. 
            // This triggers Bevy's change detection which the debounced save system monitors.
            if settings.app.input.movement_speed_multiplier != state.movement_speed_multiplier {
                settings.app.input.movement_speed_multiplier = state.movement_speed_multiplier;
            }
            if settings.core.world.hide_player != state.hide_player {
                settings.core.world.hide_player = state.hide_player;
            }
            if settings.app.performance.show_overlay != state.show_overlay {
                settings.app.performance.show_overlay = state.show_overlay;
            }
            if settings.app.performance.frame_limit_enabled != state.frame_limit_enabled {
                settings.app.performance.frame_limit_enabled = state.frame_limit_enabled;
            }
            let target_fps = FPS_PRESETS[state.fps_preset_idx];
            if settings.app.performance.target_fps != target_fps {
                settings.app.performance.target_fps = target_fps;
            }
        });

    // Write back: if egui's close button was pressed, window_open is now false.
    // Sync that back into our resource so the window stays closed next frame.
    state.open = window_open;

    // Suppress unused-variable warning (we don't need the inner response).
    let _ = response;
}
