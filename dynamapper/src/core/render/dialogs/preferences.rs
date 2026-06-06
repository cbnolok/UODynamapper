use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionTogglePreferences};
use crate::{
    configs::settings::{AntiAliasingMode, ClientTextureSource},
    core::render::{dialogs, scene::camera::UiCameraResource},
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
    /// Currently selected tab (0 = Graphics, 1 = UI).
    pub selected_tab: usize,
    pub frame_limit_enabled: bool,
    pub fps_preset_idx: usize,
    pub movement_speed_multiplier: f32,
    pub smooth_movement: bool,
    pub hide_player: bool,
    pub show_overlay: bool,
    pub free_camera: bool,
    pub ui_scale: f32,
    pub player_position_scale: f32,
    pub sysmessages_scale: f32,
    pub performance_overlay_scale: f32,
    pub cursor_position_scale: f32,
    /// VSync toggle (synced from SectGraphics).
    pub vsync: bool,
    pub anti_aliasing: AntiAliasingMode,
    pub art_texture_source: ClientTextureSource,
    pub land_texture_source: ClientTextureSource,
    pub perspective_camera: bool,
    pub enable_statics: bool,
    pub enable_static_lights: bool,
    /// Timer used to debounce applying settings that cause UI layout shifts (like UI scale).
    pub apply_timer: Timer,
}

impl Default for PreferencesDialogState {
    fn default() -> Self {
        let fps_preset_idx = FPS_PRESETS
            .iter()
            .position(|&fps| fps == DEFAULT_FPS_LIMIT)
            .unwrap_or(0);
        Self {
            open: false,
            selected_tab: 0,
            frame_limit_enabled: true,
            fps_preset_idx,
            movement_speed_multiplier: 1.0,
            smooth_movement: false,
            hide_player: false,
            show_overlay: true,
            free_camera: false,
            ui_scale: 1.0,
            player_position_scale: 1.0,
            sysmessages_scale: 1.0,
            performance_overlay_scale: 1.0,
            cursor_position_scale: 1.0,
            vsync: false,
            anti_aliasing: AntiAliasingMode::default(),
            art_texture_source: ClientTextureSource::default(),
            land_texture_source: ClientTextureSource::default(),
            perspective_camera: false,
            enable_statics: true,
            enable_static_lights: false,
            apply_timer: {
                let mut t = Timer::from_seconds(0.3, TimerMode::Once);
                t.pause();
                t
            },
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
            .add_systems(EguiPrimaryContextPass, sys_render_preferences_dialog);
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
        state.ui_scale = settings.app.window.ui_scale;
        state.player_position_scale = settings.app.window.player_position_scale;
        state.sysmessages_scale = settings.app.window.sysmessages_scale;
        state.performance_overlay_scale = settings.app.window.performance_overlay_scale;
        state.cursor_position_scale = settings.app.window.cursor_position_scale;
        state.vsync = settings.graphics.vsync;
        state.anti_aliasing = settings.graphics.anti_aliasing;
        state.art_texture_source = settings.graphics.art_texture_source;
        state.land_texture_source = settings.graphics.land_texture_source;
        state.perspective_camera = settings.app.window.perspective_camera;
        state.enable_statics = settings.world_rendering.enable_statics;
        state.enable_static_lights = settings.world_rendering.enable_static_lights;

        wireframe_config.global = settings.app.debug.map_render_wireframe;

        *initialized = true;
    }
}

fn sys_preferences_toggle(
    _trigger: On<ActionTogglePreferences>,
    mut state: ResMut<PreferencesDialogState>,
) {
    log_system_add_one_shot::<PreferencesDialogPlugin>(
        "Observer",
        "ActionTogglePreferences",
        fname!(),
    );
    state.open = !state.open;
}

fn sys_preferences_close(
    _trigger: On<ActionCloseActiveDialog>,
    mut state: ResMut<PreferencesDialogState>,
) {
    log_system_add_one_shot::<PreferencesDialogPlugin>(
        "Observer",
        "ActionCloseActiveDialog",
        fname!(),
    );
    state.open = false;
}

/// Renders the preferences dialog window with two tabs: Graphics and UI.
pub fn sys_render_preferences_dialog(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut state: ResMut<PreferencesDialogState>,
    mut framepace: ResMut<FramepaceSettings>,
    mut settings: ResMut<Settings>,
    mut windows_q: Query<&mut Window>,
    time: Res<Time>,
) {
    state.apply_timer.tick(time.delta());

    if state.apply_timer.just_finished() {
        // ---- Sync UI state to Settings resource ----
        // This is debounced via state.apply_timer to avoid "scattering" during slider manipulation.

        if (settings.as_ref().app.input.movement_speed_multiplier - state.movement_speed_multiplier)
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
        if settings.as_ref().app.window.perspective_camera != state.perspective_camera {
            settings.app.window.perspective_camera = state.perspective_camera;
        }
        if settings.as_ref().world_rendering.enable_statics != state.enable_statics {
            settings.world_rendering.enable_statics = state.enable_statics;
        }
        if settings.as_ref().world_rendering.enable_static_lights != state.enable_static_lights {
            settings.world_rendering.enable_static_lights = state.enable_static_lights;
        }
        if (settings.as_ref().app.window.ui_scale - state.ui_scale).abs() > 0.001 {
            settings.app.window.ui_scale = state.ui_scale;
        }
        if (settings.as_ref().app.window.player_position_scale - state.player_position_scale).abs()
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
        if (settings.as_ref().app.window.cursor_position_scale - state.cursor_position_scale).abs()
            > 0.001
        {
            settings.app.window.cursor_position_scale = state.cursor_position_scale;
        }
        if (settings.as_ref().app.window.sysmessages_scale - state.sysmessages_scale).abs() > 0.001
        {
            settings.app.window.sysmessages_scale = state.sysmessages_scale;
        }
    }

    if !state.open {
        return;
    }

    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    let mut window_open = state.open;

    let title = format!(
        "Options [{:?}]",
        settings.as_ref().keybindings.user_settings
    );

    let _response = egui::Window::new(title)
        .default_pos([200.0, 80.0])
        .fixed_size([320.0, 420.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            // ---- Tab bar ----
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(state.selected_tab == 0, "⚙ Graphics")
                    .clicked()
                {
                    state.selected_tab = 0;
                }
                if ui
                    .selectable_label(state.selected_tab == 1, "🖥 UI")
                    .clicked()
                {
                    state.selected_tab = 1;
                }
            });
            ui.separator();

            match state.selected_tab {
                // ==================== GRAPHICS TAB ====================
                0 => {
                    ui.heading("Performance");
                    ui.separator();

                    // ---- Frame limiter toggle ----
                    if ui
                        .checkbox(&mut state.frame_limit_enabled, "Enable frame limiter")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }

                    // ---- FPS combobox ----
                    ui.add_enabled_ui(state.frame_limit_enabled, |ui| {
                        let current_fps = FPS_PRESETS[state.fps_preset_idx];
                        let res = egui::ComboBox::from_label("Target FPS")
                            .selected_text(format!("{} fps", current_fps))
                            .show_ui(ui, |ui| {
                                for (idx, &fps) in FPS_PRESETS.iter().enumerate() {
                                    let label = format!("{} fps", fps);
                                    ui.selectable_value(&mut state.fps_preset_idx, idx, label);
                                }
                            });
                        if res.response.changed() {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });

                    // Apply frame limiter changes immediately to framepace resource
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

                    let limiter_enabled = !matches!(framepace.limiter, Limiter::Off);

                    if state.frame_limit_enabled != limiter_enabled || fps_changed {
                        framepace.limiter = if state.frame_limit_enabled {
                            Limiter::from_framerate(FPS_PRESETS[state.fps_preset_idx] as f64)
                        } else {
                            Limiter::Off
                        };
                    }

                    ui.label(
                        egui::RichText::new("Note: frame limiting also affected by VSync.")
                            .small()
                            .weak(),
                    );

                    ui.add_space(8.0);

                    // ---- VSync ----
                    if ui.checkbox(&mut state.vsync, "VSync").changed() {
                        settings.graphics.vsync = state.vsync;
                        // Apply immediately to the window's present mode
                        if let Ok(mut window) = windows_q.single_mut() {
                            window.present_mode = if state.vsync {
                                bevy::window::PresentMode::AutoVsync
                            } else {
                                bevy::window::PresentMode::AutoNoVsync
                            };
                        }
                    }

                    egui::ComboBox::from_label("Anti-Aliasing")
                        .selected_text(state.anti_aliasing.label())
                        .show_ui(ui, |ui| {
                            for mode in AntiAliasingMode::ALL {
                                ui.selectable_value(&mut state.anti_aliasing, mode, mode.label());
                            }
                        });
                    if settings.graphics.anti_aliasing != state.anti_aliasing {
                        settings.graphics.anti_aliasing = state.anti_aliasing;
                    }

                    egui::ComboBox::from_label("Art Texture Source")
                        .selected_text(state.art_texture_source.art_label())
                        .show_ui(ui, |ui| {
                            for source in ClientTextureSource::ALL {
                                ui.selectable_value(
                                    &mut state.art_texture_source,
                                    source,
                                    source.art_label(),
                                );
                            }
                        });
                    if settings.graphics.art_texture_source != state.art_texture_source {
                        settings.graphics.art_texture_source = state.art_texture_source;
                    }

                    egui::ComboBox::from_label("Land Texture Source")
                        .selected_text(state.land_texture_source.land_label())
                        .show_ui(ui, |ui| {
                            for source in ClientTextureSource::ALL {
                                ui.selectable_value(
                                    &mut state.land_texture_source,
                                    source,
                                    source.land_label(),
                                );
                            }
                        });
                    if settings.graphics.land_texture_source != state.land_texture_source {
                        settings.graphics.land_texture_source = state.land_texture_source;
                    }

                    ui.add_space(4.0);
                    ui.heading("Texture");
                    ui.separator();


                    ui.add_space(8.0);
                    ui.heading("World & Input");
                    ui.separator();

                    // ---- Movement Speed ----
                    ui.horizontal(|ui| {
                        ui.label("Move Speed:");
                        if ui
                            .add(egui::Slider::new(
                                &mut state.movement_speed_multiplier,
                                0.1..=500.0,
                            ))
                            .changed()
                        {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });

                    if ui
                        .checkbox(&mut state.smooth_movement, "Smooth movement")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }
                    ui.label(
                        egui::RichText::new("Interpolates the player between integer tile steps.")
                            .small()
                            .weak(),
                    );

                    // ---- Visibility ----
                    if ui
                        .checkbox(&mut state.hide_player, "Hide Player Object")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }
                    if ui
                        .checkbox(&mut state.show_overlay, "Show Performance Overlay")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }

                    ui.add_space(4.0);
                    if ui
                        .checkbox(&mut state.free_camera, "Free Camera Mode")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }
                    ui.label(
                        egui::RichText::new("Arrows to pan, Shift+Arrows to elevation.")
                            .small()
                            .weak(),
                    );

                    ui.add_space(4.0);
                    if ui
                        .checkbox(&mut state.perspective_camera, "Perspective Camera Mode")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }
                    if ui
                        .checkbox(&mut state.enable_statics, "Render Static Items")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }
                    if ui
                        .checkbox(&mut state.enable_static_lights, "Render Static Light Decals")
                        .changed()
                    {
                        state.apply_timer.reset();
                        state.apply_timer.unpause();
                    }
                }

                // ====================== UI TAB ========================
                1 => {
                    ui.heading("UI Scale");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("UI Scale:");
                        let res = ui.add(egui::Slider::new(&mut state.ui_scale, 0.5..=3.0));
                        if res.drag_stopped() || (res.changed() && !res.dragged()) {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });

                    ui.add_space(8.0);
                    ui.heading("Overlay Scales");
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Player Position:");
                        let res = ui.add(egui::Slider::new(
                            &mut state.player_position_scale,
                            0.5..=3.0,
                        ));
                        if res.drag_stopped() || (res.changed() && !res.dragged()) {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("System Messages:");
                        let res =
                            ui.add(egui::Slider::new(&mut state.sysmessages_scale, 0.5..=3.0));
                        if res.drag_stopped() || (res.changed() && !res.dragged()) {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Performance Overlay:");
                        let res = ui.add(egui::Slider::new(
                            &mut state.performance_overlay_scale,
                            0.5..=3.0,
                        ));
                        if res.drag_stopped() || (res.changed() && !res.dragged()) {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Cursor Position:");
                        let res = ui.add(egui::Slider::new(
                            &mut state.cursor_position_scale,
                            0.5..=3.0,
                        ));
                        if res.drag_stopped() || (res.changed() && !res.dragged()) {
                            state.apply_timer.reset();
                            state.apply_timer.unpause();
                        }
                    });
                }
                _ => {}
            }

            // (Removed immediate sync block, now handled by debounced timer at the top of the system)
        });

    // PERFORMANCE: Only update state.open if it actually changed to avoid triggering
    // Bevy's change detection on every frame, which can be expensive.
    if state.open != window_open {
        state.open = window_open;
    }
}

pub fn sys_apply_performance_settings(
    settings: Res<Settings>,
    mut framepace: ResMut<FramepaceSettings>,
) {
    if settings.is_changed() {
        log_system_add_update::<PreferencesDialogPlugin>(fname!());
        let target_fps = settings.app.performance.target_fps;
        let enabled = settings.app.performance.frame_limit_enabled;

        framepace.limiter = if enabled {
            Limiter::from_framerate(target_fps as f64)
        } else {
            Limiter::Off
        };
    }
}
