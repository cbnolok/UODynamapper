use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionToggleTeleportDialog};
use crate::core::render::scene::player::Player;
use crate::core::render::scene::world::WorldGeoData;
use crate::core::render::scene::RecomputeVisibleChunksEvent;
use crate::ingame_sysmessage_logger;
use crate::{
    core::render::{dialogs, scene::camera::UiCameraResource},
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

#[derive(Resource, Default)]
pub struct TeleportDialogState {
    pub open: bool,
    pub x: String,
    pub y: String,
    pub z: String,
    pub m: String,
}

pub struct TeleportPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TeleportPlugin);

impl Plugin for TeleportPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<TeleportDialogState>()
            .add_observer(sys_teleport_toggle)
            .add_observer(sys_teleport_close)
            .add_systems(
                EguiPrimaryContextPass,
                sys_render_teleport_dialog.run_if(in_state(AppState::InGame)),
            );
    }
}

fn sys_teleport_toggle(
    _trigger: On<ActionToggleTeleportDialog>,
    mut state: ResMut<TeleportDialogState>,
) {
    state.open = !state.open;
}

fn sys_teleport_close(
    _trigger: On<ActionCloseActiveDialog>,
    mut state: ResMut<TeleportDialogState>,
) {
    state.open = false;
}

pub fn sys_render_teleport_dialog(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut state: ResMut<TeleportDialogState>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    world_geo_data: Res<WorldGeoData>,
) {
    if !state.open {
        return;
    }

    // Try to get the egui context - if it fails, skip rendering this frame
    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    egui::Window::new("Teleport (GoTo)")
        .default_pos([400.0, 400.0])
        .fixed_size([100.0, 160.0])
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("X:");
                ui.text_edit_singleline(&mut state.x);
            });
            ui.horizontal(|ui| {
                ui.label("Y:");
                ui.text_edit_singleline(&mut state.y);
            });
            ui.horizontal(|ui| {
                ui.label("Z:");
                ui.text_edit_singleline(&mut state.z);
            });
            ui.horizontal(|ui| {
                ui.label("M:");
                ui.text_edit_singleline(&mut state.m);
            });
            if ui.button("Teleport").clicked() {
                if let (Ok(x), Ok(y), Ok(z), Ok(m)) = (
                    state.x.parse::<u16>(),
                    state.y.parse::<u16>(),
                    state.z.parse::<i8>(),
                    state.m.parse::<u8>(),
                ) {
                    if let Ok((mut player, mut transform)) = player_q.single_mut() {
                        // Validate target coordinates
                        if let Some(meta) = world_geo_data.maps.get(&(m as u32)) {
                            if x as u32 >= meta.width || y as u32 >= meta.height {
                                crate::ingame_sysmessage_logger::error(format!(
                                    "Target coordinates [{}, {}] out of bounds for map {} ({}x{})",
                                    x, y, m, meta.width, meta.height
                                ));
                                return;
                            }
                        } else {
                            crate::ingame_sysmessage_logger::error(format!(
                                "Invalid map ID: {}",
                                m
                            ));
                            return;
                        }

                        let uo_pos = UOVec4::new(x, y, z, m);
                        player.current_pos = Some(uo_pos);
                        let bevy_pos = uo_pos.to_bevy_vec3_ignore_map();
                        transform.translation = bevy_pos;
                        // Always force a full recompute — same-map teleports must also
                        // despawn the old chunks and spawn the new visible set.
                        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});

                        ingame_sysmessage_logger::normal(format!(
                            "Teleported to [{}, {}, {}, {}]",
                            x, y, z, m
                        ));
                        state.open = false;
                    }
                }
            }
        });
}
