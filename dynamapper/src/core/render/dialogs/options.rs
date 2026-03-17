// Options dialog (egui window)

use crate::{
    core::render::{dialogs::get_egui_context_ready, scene::camera::UiCameraResource},
    prelude::*,
};
use bevy::{pbr::wireframe::WireframeConfig, prelude::*};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use bevy_framepace::{FramepaceSettings, Limiter};

/// Preset FPS values offered in the combobox.
/// Using a fixed list keeps the UI simple and avoids having a free-form number input
/// that could produce unrealistic or unstable values.
const FPS_PRESETS: &[u32] = &[15, 30, 60, 75, 120, 144, 165, 240];

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
    /// Whether the camera is free.
    pub free_camera: bool,
    /// Egui scale factor.
    pub egui_scale: f32,
    /// Overlay scale factor.
    pub overlay_scale: f32,
    /// Manual terrain mesh reduction factor (1/2/4).
    pub chunk_mesh_reduction_factor: u32,
    /// Multiplier for visible-area safety margin (higher = more chunks spawned).
    pub chunk_visibility_overscan: f32,
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
            free_camera: false,
            egui_scale: 1.0,
            overlay_scale: 1.0,
            chunk_mesh_reduction_factor: 1,
            chunk_visibility_overscan: 1.6,
        }
    }
}

// ---- Plugin ----

pub struct OptionsDialogPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(OptionsDialogPlugin);

impl Plugin for OptionsDialogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OptionsDialogState>()
            .add_systems(
                Update,
                (
                    sys_toggle_options_dialog.run_if(in_state(AppState::InGame)),
                    sys_sync_settings_to_state.run_if(in_state(AppState::InGame)),
                    sys_apply_performance_settings.run_if(in_state(AppState::InGame)),
                ),
            )
            .add_systems(EguiPrimaryContextPass, sys_render_options_dialog);
    }
}

/// Syncs from Settings resource to UI state.
/// We sync if it's the first initialization, or if the settings resource changed while the dialog is closed.
fn sys_sync_settings_to_state(
    settings: Res<Settings>,
    mut state: ResMut<OptionsDialogState>,
    mut initialized: Local<bool>,
    mut wireframe_config: ResMut<WireframeConfig>,
) {
    let should_sync = !*initialized || (!state.open && settings.is_changed());

    if should_sync {
        state.movement_speed_multiplier = settings.app.input.movement_speed_multiplier;
        state.hide_player = settings.core.world.hide_player;
        state.show_overlay = settings.app.performance.show_overlay;
        state.frame_limit_enabled = settings.app.performance.frame_limit_enabled;
        state.fps_preset_idx = FPS_PRESETS
            .iter()
            .position(|&fps| fps == settings.app.performance.target_fps)
            .unwrap_or(0);
        state.chunk_mesh_reduction_factor = settings.app.performance.chunk_mesh_reduction_factor;
        state.chunk_visibility_overscan = settings.app.performance.chunk_visibility_overscan;
        state.free_camera = settings.app.window.free_camera;
        state.egui_scale = settings.app.window.egui_scale;
        state.overlay_scale = settings.app.window.overlay_scale;

        // Also apply wireframe setting which isn't in the dialog yet but is in settings
        wireframe_config.global = settings.app.debug.map_render_wireframe;

        *initialized = true;
    }
}

// ---- Systems ----

/// Toggles the options dialog; also allows Escape to close it.
fn sys_toggle_options_dialog(
    keyboard: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut state: ResMut<OptionsDialogState>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    if let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) {
        if ctx.wants_keyboard_input() {
            //return;
        }
    }

    if keyboard.just_pressed(settings.keybindings.user_settings) {
        state.open = !state.open;
    }
    // Pressing Escape closes the dialog if it is open.
    if keyboard.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
    }
}

/// Renders the options dialog window and applies any changes to the relevant resources.
pub fn sys_render_options_dialog(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut state: ResMut<OptionsDialogState>,
    mut framepace: ResMut<FramepaceSettings>,
    mut settings: ResMut<Settings>,
) {
    if !state.open {
        return;
    }

    // Try to get the egui context - if it fails, skip rendering this frame
    let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    // — Borrow fix —
    // `egui::Window::open()` mutably borrows the bool for the lifetime of the show()
    // call, which clashes with re-borrowing `state` inside the closure.
    // Solution: stage the value in a local, pass a reference to that local, then
    // write the (possibly changed by egui's own close button) value back afterward.
    let mut window_open = state.open;

    let title = format!("Options [{:?}]", settings.as_ref().keybindings.user_settings);

    let response = egui::Window::new(title)
        .default_pos([200.0, 80.0])
        .fixed_size([300.0, 400.0])
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
                ui.add(egui::Slider::new(
                    &mut state.movement_speed_multiplier,
                    0.1..=500.0,
                ));
            });

            // ---- Visibility/Graphics ----
            ui.checkbox(&mut state.hide_player, "Hide Player Object");
            ui.checkbox(&mut state.show_overlay, "Show Performance Overlay");

            ui.horizontal(|ui| {
                ui.label("Mesh Quality Reduction:");
                egui::ComboBox::from_id_salt("mesh_quality_reduction")
                    .selected_text(match state.chunk_mesh_reduction_factor {
                        2 => "1/2",
                        4 => "1/4",
                        _ => "1/1",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut state.chunk_mesh_reduction_factor, 1, "1/1 (full)");
                        ui.selectable_value(&mut state.chunk_mesh_reduction_factor, 2, "1/2");
                        ui.selectable_value(&mut state.chunk_mesh_reduction_factor, 4, "1/4");
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Chunk Visibility Overscan:");
                ui.add(egui::Slider::new(
                    &mut state.chunk_visibility_overscan,
                    1.0..=2.5,
                ));
            });

            ui.add_space(4.0);
            ui.checkbox(&mut state.free_camera, "Free Camera Mode");
            ui.label(egui::RichText::new("Arrows to pan, Shift+Arrows to elevation.").small().weak());

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Egui Scale:");
                if ui.add(egui::Slider::new(&mut state.egui_scale, 0.5..=3.0)).changed() {
                    settings.app.window.egui_scale = state.egui_scale;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Overlay Scale:");
                if ui.add(egui::Slider::new(&mut state.overlay_scale, 0.5..=3.0)).changed() {
                    settings.app.window.overlay_scale = state.overlay_scale;
                }
            });

            // ---- Sync UI state to Settings resource ----
            // We use .as_ref() for comparisons to avoid triggering change detection
            // unless we actually write a new value. This prevents the debounced save
            // timer from being reset every frame.
            if (settings.as_ref().app.input.movement_speed_multiplier
                - state.movement_speed_multiplier).abs() > 0.001
            {
                settings.app.input.movement_speed_multiplier = state.movement_speed_multiplier;
            }
            if settings.as_ref().core.world.hide_player != state.hide_player {
                settings.core.world.hide_player = state.hide_player;
            }
            if settings.as_ref().app.performance.show_overlay != state.show_overlay {
                settings.app.performance.show_overlay = state.show_overlay;
            }
            if settings.as_ref().app.performance.frame_limit_enabled != state.frame_limit_enabled {
                settings.app.performance.frame_limit_enabled = state.frame_limit_enabled;
            }
            let target_fps = FPS_PRESETS[state.fps_preset_idx];
            if settings.as_ref().app.performance.target_fps != target_fps {
                settings.app.performance.target_fps = target_fps;
            }
            if settings.as_ref().app.performance.chunk_mesh_reduction_factor
                != state.chunk_mesh_reduction_factor
            {
                settings.app.performance.chunk_mesh_reduction_factor =
                    state.chunk_mesh_reduction_factor;
            }
            if (settings.as_ref().app.performance.chunk_visibility_overscan
                - state.chunk_visibility_overscan)
                .abs()
                > 0.001
            {
                settings.app.performance.chunk_visibility_overscan =
                    state.chunk_visibility_overscan;
            }
            if settings.as_ref().app.window.free_camera != state.free_camera {
                settings.app.window.free_camera = state.free_camera;
            }
            if (settings.as_ref().app.window.egui_scale - state.egui_scale).abs() > 0.001 {
                settings.app.window.egui_scale = state.egui_scale;
            }
            if (settings.as_ref().app.window.overlay_scale - state.overlay_scale).abs() > 0.001 {
                settings.app.window.overlay_scale = state.overlay_scale;
            }
        });

    // Write back: if egui's close button was pressed, window_open is now false.
    // Sync that back into our resource so the window stays closed next frame.
    state.open = window_open;

    // Suppress unused-variable warning (we don't need the inner response).
    let _ = response;
}

/// Applies performance settings (frame limiter) from the Settings resource to the FramepaceSettings resource.
pub fn sys_apply_performance_settings(
    settings: Res<Settings>,
    mut framepace: ResMut<FramepaceSettings>,
) {
    if settings.is_changed() {
        let target_fps = settings.app.performance.target_fps;
        let enabled = settings.app.performance.frame_limit_enabled;

        framepace.limiter = if enabled {
            Limiter::from_framerate(target_fps as f64)
        } else {
            Limiter::Off
        };
    }
}
