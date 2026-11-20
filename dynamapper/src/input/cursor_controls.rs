//! What a left click on the world does: nothing yet in select mode, and in teleport mode it moves the player to the
//! terrain cell under the cursor. Ctrl+T switches; the cursor overlay shows the mode.
use crate::input::input_actions::ActionToggleCursorTeleportMode;
use crate::input::{InjectedPointer, effective_cursor_position};
use crate::render::picking::cursor_map_id;
use crate::render::picking::terrain_cursor::{TerrainCursor, TerrainCursorQuery};
use crate::render::scene::RecomputeVisibleChunksEvent;
use crate::world::{maps::WorldGeoData, player::Player};
use crate::ingame_sysmessage_logger;
use crate::prelude::*;
use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorMode {
    #[default]
    Select,
    Teleport,
}

#[derive(Resource, Default)]
pub struct CursorBehavior {
    pub mode: CursorMode,
}

impl CursorMode {
    pub fn label(self) -> &'static str {
        match self {
            CursorMode::Select => "Select",
            CursorMode::Teleport => "Teleport",
        }
    }
}

pub struct CursorControlsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(CursorControlsPlugin);

impl Plugin for CursorControlsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<CursorBehavior>()
            .add_observer(sys_toggle_cursor_mode)
            .add_systems(Update, sys_teleport_on_click.after(TerrainCursorQuery).run_if(in_state(AppState::InGame)));
    }
}

fn sys_toggle_cursor_mode(
    _trigger: On<ActionToggleCursorTeleportMode>,
    mut cursor: ResMut<CursorBehavior>,
) {
    cursor.mode = match cursor.mode {
        CursorMode::Select => CursorMode::Teleport,
        CursorMode::Teleport => CursorMode::Select,
    };
    ingame_sysmessage_logger::normal(format!("Cursor mode: {}", cursor.mode.label()));
}

fn sys_teleport_on_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
    cursor: Res<CursorBehavior>,
    settings: Res<Settings>,
    mut egui_contexts: EguiContexts,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    world_geo_data: Res<WorldGeoData>,
    terrain: Res<TerrainCursor>,
) {
    if cursor.mode != CursorMode::Teleport {
        return;
    }

    // If egui wants pointer input, don't teleport
    if egui_contexts.ctx_mut().is_ok_and(|ctx| ctx.egui_wants_pointer_input()) {
        return;
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(window) = windows.single().ok() else {
        return;
    };

    let cursor_pos = match effective_cursor_position(window, injected_pointer.as_deref()) {
        Some(pos) => pos,
        None => return,
    };

    let Some(hit) = terrain.hit.filter(|_| terrain.pointer == Some(cursor_pos)) else { return; };
    if let Ok((mut player, mut transform)) = player_q.single_mut() {
        let Some(map) = cursor_map_id(Some(&player), &settings) else { return; };
        if hit.logical.m != map { return; }
        if crate::world::teleport::teleport_player(&mut player, &mut transform, hit.logical, &world_geo_data).is_err() { return; }
        // Always force a full recompute so the new visible area loads immediately.
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
        ingame_sysmessage_logger::normal(format!(
            "Teleported to [{}, {}, {}, {}] via cursor",
            hit.logical.x, hit.logical.y, hit.logical.z, hit.logical.m
        ));
    }
}
