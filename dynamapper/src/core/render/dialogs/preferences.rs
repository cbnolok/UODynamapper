use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionTogglePreferences};
use crate::{
    core::render::{dialogs::get_egui_context_ready, scene::camera::UiCameraResource},
    prelude::*,
};
use bevy::{pbr::wireframe::WireframeConfig, prelude::*};
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use bevy_framepace::{FramepaceSettings, Limiter};

const FPS_PRESETS: &[u32] = &[15, 30, 60, 75, 120, 144, 165, 240];
const DEFAULT_FPS_LIMIT: u32 = 60;

#[derive(Resource)]
pub struct PreferencesDialogState {
    pub open: bool,
    pub frame_limit_enabled: bool,
    pub fps_preset_idx: usize,
    pub movement_speed_multiplier: f32,
    pub smooth_movement: bool,
    pub hide_player: bool,
    pub show_overlay: bool,
    pub free_camera: bool,
    pub egui_scale: f32,
    pub hot_reload_enabled: bool,
    pub player_position_scale: f32,
    pub sysmessages_scale: f32,
    pub performance_overlay_scale: f32,
}

impl Default for PreferencesDialogState {
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
            smooth_movement: false,
            hide_player: false,
            show_overlay: true,
            free_camera: false,
            egui_scale: 1.0,
            hot_reload_enabled: false,
            player_position_scale: 1.0,
            sysmessages_scale: 1.0,
            performance_overlay_scale: 1.0,
        }
    }
}

pub struct PreferencesDialogPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PreferencesDialogPlugin);

impl Plugin for PreferencesDialogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PreferencesDialogState>()
            .add_observer(sys_preferences_toggle)
            .add_observer(sys_preferences_close)
            .add_systems(
                Update,
                (
                    sys_sync_settings_to_state.run_if(in_state(AppState::InGame)),
                    sys_apply_performance_settings.run_if(in_state(AppState::InGame)),
                ),
            )
            .add_systems(EguiPrimaryContextPass, sys_render_options_dialog);
    }
}

fn sys_sync_settings_to_state(
    settings: Res<Settings>,
    mut state: ResMut<PreferencesDialogState>,
    mut initialized: Local<bool>,
    mut wireframe_config: ResMut<WireframeConfig>,
) {
    let should_sync = !*initialized || (!state.open && settings.is_changed());

    if should_sync {
        state.movement_speed_multiplier = settings.app.input.movement_speed_multiplier;
        state.smooth_movement = settings.app.input.smooth_movement;
        state.hide_player = settings.core.world.hide_player;
        state.show_overlay = settings.app.performance.show_overlay;
        state.frame_limit_enabled = settings.app.performance.frame_limit_enabled;
        state.fps_preset_idx = FPS_PRESETS
            .iter()
            .position(|&fps| fps == settings.app.performance.target_fps)
            .unwrap_or(0);
        state.free_camera = settings.app.window.free_camera;
        state.egui_scale = settings.app.window.egui_scale;
        state.hot_reload_enabled = settings.app.debug.hot_reload_enabled;
        state.player_position_scale = settings.app.window.player_position_scale;
        state.sysmessages_scale = settings.app.window.sysmessages_scale;
        state.performance_overlay_scale = settings.app.window.performance_overlay_scale;

        wireframe_config.global = settings.app.debug.map_render_wireframe;

        *initialized = true;
    }
}

fn sys_preferences_toggle(
    _trigger: On<ActionTogglePreferences>,
    mut state: ResMut<PreferencesDialogState>,
) {
    state.open = !state.open;
}

fn sys_preferences_close(
    _trigger: On<ActionCloseActiveDialog>,
    mut state: ResMut<PreferencesDialogState>,
) {
    state.open = false;
}

pub fn sys_render_options_dialog(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut state: ResMut<PreferencesDialogState>,
    mut framepace: ResMut<FramepaceSettings>,
    mut settings: ResMut<Settings>,
) {
    if !state.open {
        return;
    }

    let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    let mut window_open = state.open;

    let title = format!(
        "Options [{:?}]",
        settings.as_ref().keybindings.user_settings
    );

    let response = egui::Window::new(title)
        .default_pos([200.0, 80.0])
        .fixed_size([300.0, 400.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            ui.heading("Performance");
            ui.separator();

            let prev_enabled = state.frame_limit_enabled;
            ui.checkbox(&mut state.frame_limit_enabled, "Enable frame limiter");

            ui.add_enabled_ui(state.frame_limit_enabled, |ui| {
                let current_fps = FPS_PRESETS[state.fps_preset_idx];
                egui::ComboBox::from_label("Target FPS")
                    .selected_text(format!("{} fps", current_fps))
                    .show_ui(ui, |ui| {
                        for (idx, &fps) in FPS_PRESETS.iter().enumerate() {
                            let label = format!("{} fps", fps);
                            ui.selectable_value(&mut state.fps_preset_idx, idx, label);
                        }
                    });
            });

            let fps_changed = state.fps_preset_idx != {
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
                    Limiter::from_framerate(FPS_PRESETS[state.fps_preset_idx] as f64)
                } else {
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
            ui.horizontal(|ui| {
                ui.label("Hot-reload settings:");
                if ui
                    .checkbox(&mut state.hot_reload_enabled, "Enable")
                    .changed()
                {
                    settings.app.debug.hot_reload_enabled = state.hot_reload_enabled;
                }
            });
            ui.add_space(8.0);
            ui.heading("World & Input");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Move Speed:");
                ui.add(egui::Slider::new(
                    &mut state.movement_speed_multiplier,
                    0.1..=500.0,
                ));
            });

            ui.checkbox(&mut state.smooth_movement, "Smooth movement");
            ui.label(
                egui::RichText::new("Interpolates the player between integer tile steps.")
                    .small()
                    .weak(),
            );

            ui.checkbox(&mut state.hide_player, "Hide Player Object");
            ui.checkbox(&mut state.show_overlay, "Show Performance Overlay");

            ui.add_space(4.0);
            ui.checkbox(&mut state.free_camera, "Free Camera Mode");
            ui.label(
                egui::RichText::new("Arrows to pan, Shift+Arrows to elevation.")
                    .small()
                    .weak(),
            );

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Egui Scale:");
                if ui
                    .add(egui::Slider::new(&mut state.egui_scale, 0.5..=3.0))
                    .changed()
                {
                    settings.app.window.egui_scale = state.egui_scale;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Player Position Scale:");
                if ui
                    .add(egui::Slider::new(
                        &mut state.player_position_scale,
                        0.5..=3.0,
                    ))
                    .changed()
                {
                    settings.app.window.player_position_scale = state.player_position_scale;
                }
            });
            ui.horizontal(|ui| {
                ui.label("System Messages Scale:");
                if ui
                    .add(egui::Slider::new(&mut state.sysmessages_scale, 0.5..=3.0))
                    .changed()
                {
                    settings.app.window.sysmessages_scale = state.sysmessages_scale;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Performance Overlay Scale:");
                if ui
                    .add(egui::Slider::new(
                        &mut state.performance_overlay_scale,
                        0.5..=3.0,
                    ))
                    .changed()
                {
                    settings.app.window.performance_overlay_scale = state.performance_overlay_scale;
                }
            });

            if (settings.as_ref().app.input.movement_speed_multiplier
                - state.movement_speed_multiplier)
                .abs()
                > 0.001
            {
                settings.app.input.movement_speed_multiplier = state.movement_speed_multiplier;
            }
            if settings.as_ref().app.input.smooth_movement != state.smooth_movement {
                settings.app.input.smooth_movement = state.smooth_movement;
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
            if settings.as_ref().app.window.free_camera != state.free_camera {
                settings.app.window.free_camera = state.free_camera;
            }
            if (settings.as_ref().app.window.egui_scale - state.egui_scale).abs() > 0.001 {
                settings.app.window.egui_scale = state.egui_scale;
            }
            if (settings.as_ref().app.window.player_position_scale - state.player_position_scale)
                .abs()
                > 0.001
            {
                settings.app.window.player_position_scale = state.player_position_scale;
            }
            if (settings.as_ref().app.window.performance_overlay_scale
                - state.performance_overlay_scale)
                .abs()
                > 0.001
            {
                settings.app.window.performance_overlay_scale = state.performance_overlay_scale;
            }
            if (settings.as_ref().app.window.sysmessages_scale - state.sysmessages_scale).abs()
                > 0.001
            {
                settings.app.window.sysmessages_scale = state.sysmessages_scale;
            }
        });

    state.open = window_open;
    let _ = response;
}

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
