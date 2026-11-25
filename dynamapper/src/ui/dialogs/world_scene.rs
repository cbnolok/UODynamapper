//! The World Scene window: the state the player or the server changes at run time, apart from the art direction of
//! the Shading Controls window.
//!
//! - Time of day and light level. Each render style has a preset per time of day, so the Time of Day buttons load the
//!   active style's preset of that period, as they did in the Shading window. The light level is the UO global light
//!   level a server sends (packet 0x4F), 0 full day to `MAX_CLIENT_LIGHT_LEVEL`, and drives the same night lighting
//!   as before (`render::lighting::client_night`): Classic multiplies the frame by (32 - L) / 32 before its light.mul
//!   lights take the darkness back, the other styles composite the frame with the night map and the light sprites at
//!   the clients' level 1 - L / 31. It stays a value of the active preset (`client_lighting.light_level`), so each
//!   style and time of day keeps its own, and closing this window saves the presets as closing Shading does.
//! - The player's look: hidden, the marker cube, a human or a human on a horse (`core.world`, saved with the core
//!   settings). The rider plays the mounted actions of `render::mobiles`.
//! - The player's z: set live, it moves the player up or down on its cell as the altitude keys do, so the z-view cut
//!   ("Show Only the Player's Floor", `world::z_view`) and the player's cave (`world::caves`) follow it. The next step
//!   of a drawn body takes its z from the ground again. The position, z included, is saved with the session.

use crate::input::input_actions::{ActionCloseActiveDialog, ActionToggleWorldScene};
use crate::render::lighting::client_night::{
    classic_light_level_multiplier, client_light_level_weight, CLIENT_DAY_LIGHT_LEVEL, MAX_CLIENT_LIGHT_LEVEL,
};
use crate::render::terrain::mesh_material::LandShaderModePresets;
use crate::render::uniform_state::UniformState;
use crate::render::view::camera::UiCameraResource;
use crate::settings::shader_presets::{build_save_presets, save_shader_settings};
use crate::ui::dialogs;
use crate::ui::dialogs::shading::{
    active_render_style, store_active_and_apply_render_preset, ShaderPresetPeriod, PRESET_PERIOD_BUTTONS,
};
use crate::world::player::Player;
use crate::prelude::*;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

/// The human body a server sends for a male character.
pub const HUMAN_MALE_BODY: u16 = 400;
/// The human body a server sends for a female character.
pub const HUMAN_FEMALE_BODY: u16 = 401;
/// The brown horse, the standard mount (body 0xC8).
pub const HORSE_BODY: u16 = 200;

/// The light level ServUO and RunUO send at night (the user's choice over 10, 2026-11-18).
pub const SERVER_NIGHT_LIGHT_LEVEL: u32 = 12;
/// The light level ServUO sends in dungeons.
pub const SERVER_DUNGEON_LIGHT_LEVEL: u32 = 26;
/// The light level quick buttons: `(label, level)`.
const LIGHT_LEVEL_BUTTONS: [(&str, u32); 3] = [
    ("Day", CLIENT_DAY_LIGHT_LEVEL),
    ("Night", SERVER_NIGHT_LIGHT_LEVEL),
    ("Dungeon", SERVER_DUNGEON_LIGHT_LEVEL),
];

/// The z range of a UO position.
const PLAYER_Z_RANGE: std::ops::RangeInclusive<i32> = i8::MIN as i32..=i8::MAX as i32;

const WINDOW_DEFAULT_POS: [f32; 2] = [400.0, 120.0];
const WINDOW_DEFAULT_WIDTH: f32 = 440.0;
const SECTION_SPACING: f32 = 8.0;

const TIME_OF_DAY_TIP: &str =
    "Loads the render style's preset for the time of day, with its light level. Your edits to the current preset are kept until you save or undo in Shading Controls.";
const LIGHT_LEVEL_TIP: &str =
    "The server's global light level: 0 is full day (no change), 12 a night (ServUO's), 26 a dungeon; the client applies up to 30. Every style dims by it as its client does: Classic multiplies the frame by (32 - level) / 32, with the light.mul lights taking that darkness back; the others composite the frame with the KR and Enhanced clients' night map at 1 - level / 31, with their light sprites. Kept per style and time of day.";
const SCENE_LIGHT_TIP: &str =
    "The share of the day's light the level leaves where no light source shines: (32 - level) / 32 in Classic, 1 - level / 31 in the other styles, before their night map tint.";
const PLAYER_Z_TIP: &str =
    "The player's z, from -128 to 127. With Show Only the Player's Floor, what is above it is hidden, and under the land it shows the cave the player stands in. Walking takes the z from the ground again.";
const SHOW_ONLY_PLAYER_FLOOR_TIP: &str =
    "Hides the roofs and upper floors over the player, and the land over a cave the player is in, as the clients do. The same setting as in Options.";

/// Whether the World Scene window is shown, and whether it changed the shader presets since it opened.
#[derive(Resource, Default)]
pub struct WorldSceneUiState {
    pub open: bool,
    presets_edited: bool,
}

pub struct WorldScenePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(WorldScenePlugin);

impl Plugin for WorldScenePlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<WorldSceneUiState>()
            .add_observer(sys_world_scene_toggle)
            .add_observer(sys_world_scene_close)
            .add_systems(EguiPrimaryContextPass, sys_render_world_scene.run_if(in_state(AppState::InGame)));
    }
}

fn sys_world_scene_toggle(_trigger: On<ActionToggleWorldScene>, mut state: ResMut<WorldSceneUiState>) {
    log_system_add_one_shot::<WorldScenePlugin>("Observer", "ActionToggleWorldScene", fname!());
    state.open = !state.open;
}

fn sys_world_scene_close(_trigger: On<ActionCloseActiveDialog>, mut state: ResMut<WorldSceneUiState>) {
    log_system_add_one_shot::<WorldScenePlugin>("Observer", "ActionCloseActiveDialog", fname!());
    state.open = false;
}

/// The player's looks the window offers, each a set of `core.world` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayerLook {
    Hidden,
    Marker,
    Human,
    HumanFemale,
    HumanOnHorse,
}

/// The look buttons: `(label, look, tooltip)`.
const PLAYER_LOOKS: [(&str, PlayerLook, &str); 5] = [
    ("Hidden", PlayerLook::Hidden, "No player is drawn."),
    ("Marker", PlayerLook::Marker, "The marker cube."),
    ("Human", PlayerLook::Human, "A man (body 400) on foot."),
    ("Human (female)", PlayerLook::HumanFemale, "A woman (body 401) on foot."),
    ("Human on Horse", PlayerLook::HumanOnHorse, "A man (body 400) riding a horse (body 200), with the mounted actions."),
];

impl PlayerLook {
    /// The look the settings hold, or `None` for another body or mount (set through the control channel).
    fn of(world: &SectWorld) -> Option<Self> {
        if world.hide_player {
            return Some(Self::Hidden);
        }
        match (world.player_body, world.player_mount_body) {
            (0, _) => Some(Self::Marker),
            (HUMAN_MALE_BODY, 0) => Some(Self::Human),
            (HUMAN_FEMALE_BODY, 0) => Some(Self::HumanFemale),
            (HUMAN_MALE_BODY, HORSE_BODY) => Some(Self::HumanOnHorse),
            _ => None,
        }
    }

    /// Sets the settings to this look. Hiding the player keeps the body it had.
    fn apply(self, world: &mut SectWorld) {
        let (body, mount) = match self {
            Self::Hidden => {
                world.hide_player = true;
                return;
            }
            Self::Marker => (0, 0),
            Self::Human => (HUMAN_MALE_BODY, 0),
            Self::HumanFemale => (HUMAN_FEMALE_BODY, 0),
            Self::HumanOnHorse => (HUMAN_MALE_BODY, HORSE_BODY),
        };
        world.hide_player = false;
        world.player_body = body;
        world.player_mount_body = mount;
    }
}

/// A light level from the slider, within what the client applies.
fn clamp_light_level(level: u32) -> u32 {
    level.min(MAX_CLIENT_LIGHT_LEVEL)
}

/// The share of the day's light a light level leaves without light sources: the Classic client's frame multiplier,
/// or the KR and Enhanced clients' composite level (`render::lighting::client_night`).
fn scene_light_factor(level: u32, classic_rule: bool) -> f32 {
    if classic_rule {
        classic_light_level_multiplier(level)
    } else {
        client_light_level_weight(level)
    }
}

/// A z from the slider, within a UO position's range.
fn clamp_player_z(z: i32) -> i8 {
    z.clamp(*PLAYER_Z_RANGE.start(), *PLAYER_Z_RANGE.end()) as i8
}

/// Moves the player to `z` on its cell, as the altitude keys do: the position, and the transform's height at once.
fn set_player_z(player: &mut Player, transform: &mut Transform, z: i8) {
    let Some(pos) = player.current_pos.as_mut() else {
        return;
    };
    pos.z = z;
    transform.translation.y = scale_uo_z_to_bevy_units(f32::from(z));
}

#[allow(clippy::too_many_arguments)]
fn sys_render_world_scene(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut state: ResMut<WorldSceneUiState>,
    mut u: ResMut<UniformState>,
    mut shader_presets: ResMut<LandShaderModePresets>,
    mut settings: ResMut<Settings>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
    mut was_open: Local<bool>,
) {
    // One more pass after closing, to save the presets.
    if !state.open && !*was_open {
        return;
    }
    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    let mut window_open = state.open;
    let mut presets_edited = false;
    let mut look_choice = None;
    let mut z_choice = None;
    let mut show_only_player_floor = settings.world_rendering.show_only_player_floor;
    let player_z = player_q.single().ok().and_then(|(player, _)| player.current_pos).map(|pos| pos.z);

    egui::Window::new(format!("World Scene [{:?}]", settings.keybindings.world_scene))
        .default_pos(WINDOW_DEFAULT_POS)
        .default_width(WINDOW_DEFAULT_WIDTH)
        .open(&mut window_open)
        .resizable(true)
        .show(ctx, |ui| {
            ui.strong("Time and light");
            ui.horizontal_wrapped(|ui| {
                ui.label("Time of Day:");
                let style = active_render_style(&u);
                let cur_period = ShaderPresetPeriod::from_active_key(&u.active_preset);
                for (label, period) in PRESET_PERIOD_BUTTONS {
                    if ui.selectable_label(cur_period == period, label).on_hover_text(TIME_OF_DAY_TIP).clicked() {
                        store_active_and_apply_render_preset(&mut u, &mut shader_presets, style, period);
                        presets_edited = true;
                    }
                }
            });
            let classic_rule = u.dims_by_classic_rule();
            match u.client_lighting.map(|lighting| lighting.light_level) {
                Some(current) => {
                    let mut level = current;
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Slider::new(&mut level, 0..=MAX_CLIENT_LIGHT_LEVEL).text("Light Level"))
                            .on_hover_text(LIGHT_LEVEL_TIP);
                        for (label, preset_level) in LIGHT_LEVEL_BUTTONS {
                            if ui.selectable_label(level == preset_level, format!("{label} {preset_level}"))
                                .on_hover_text(LIGHT_LEVEL_TIP).clicked()
                            {
                                level = preset_level;
                            }
                        }
                    });
                    ui.label(format!("Scene light: {:.0} % of the day's", scene_light_factor(level, classic_rule) * 100.0))
                        .on_hover_text(SCENE_LIGHT_TIP);
                    let level = clamp_light_level(level);
                    if level != current {
                        if let Some(lighting) = u.client_lighting.as_mut() {
                            lighting.light_level = level;
                        }
                        u.dirty = true;
                        presets_edited = true;
                    }
                }
                None => {
                    ui.weak("This preset has no light level.");
                }
            }
            ui.small(format!("Kept in the preset {}: each style and time of day has its own.", u.active_preset));

            ui.add_space(SECTION_SPACING);
            ui.separator();
            ui.strong("Player");
            let current_look = PlayerLook::of(&settings.core.world);
            ui.horizontal_wrapped(|ui| {
                for (label, look, tip) in PLAYER_LOOKS {
                    if ui.selectable_label(current_look == Some(look), label).on_hover_text(tip).clicked()
                        && current_look != Some(look)
                    {
                        look_choice = Some(look);
                    }
                }
            });
            if current_look.is_none() {
                let world = &settings.core.world;
                ui.weak(match world.player_mount_body {
                    0 => format!("Now body {}.", world.player_body),
                    mount => format!("Now body {} on body {mount}.", world.player_body),
                });
            }

            ui.add_space(SECTION_SPACING);
            ui.separator();
            ui.strong("Height");
            match player_z {
                Some(current) => {
                    let mut z = i32::from(current);
                    ui.add(egui::Slider::new(&mut z, PLAYER_Z_RANGE).text("Player Z")).on_hover_text(PLAYER_Z_TIP);
                    let z = clamp_player_z(z);
                    if z != current {
                        z_choice = Some(z);
                    }
                }
                None => {
                    ui.weak("No player position yet.");
                }
            }
            ui.checkbox(&mut show_only_player_floor, "Show Only the Player's Floor")
                .on_hover_text(SHOW_ONLY_PLAYER_FLOOR_TIP);
        });

    if let Some(look) = look_choice {
        look.apply(&mut settings.core.world);
    }
    if show_only_player_floor != settings.world_rendering.show_only_player_floor {
        settings.world_rendering.show_only_player_floor = show_only_player_floor;
    }
    if let (Some(z), Ok((mut player, mut transform))) = (z_choice, player_q.single_mut()) {
        set_player_z(&mut player, &mut transform, z);
    }
    state.presets_edited |= presets_edited;
    if state.open != window_open {
        state.open = window_open;
    }

    // Closing the window saves the presets it changed, as closing Shading Controls does.
    if *was_open && !state.open && state.presets_edited {
        let save_presets = build_save_presets(&u, &shader_presets);
        save_shader_settings(&save_presets);
        u.saved_snapshot = Some(u.snapshot());
        state.presets_edited = false;
    }
    *was_open = state.open;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(hide_player: bool, player_body: u16, player_mount_body: u16) -> SectWorld {
        SectWorld { start_p: UOVec4::new(0, 0, 0, 0), hide_player, player_body, player_mount_body }
    }

    /// Every look sets the settings that read back as that look, through a save and a load of the core settings.
    #[test]
    fn every_player_look_reads_back_from_the_saved_settings() {
        for (_, look, _) in PLAYER_LOOKS {
            let mut settings = world(false, HUMAN_MALE_BODY, 0);
            look.apply(&mut settings);
            let saved = toml::to_string(&settings).unwrap();
            let loaded: SectWorld = toml::from_str(&saved).unwrap();
            assert_eq!(PlayerLook::of(&loaded), Some(look), "{look:?}");
        }
    }

    #[test]
    fn hiding_keeps_the_body_and_showing_again_sets_it() {
        let mut settings = world(false, HUMAN_MALE_BODY, HORSE_BODY);
        PlayerLook::Hidden.apply(&mut settings);
        assert!(settings.hide_player);
        assert_eq!((settings.player_body, settings.player_mount_body), (HUMAN_MALE_BODY, HORSE_BODY));
        PlayerLook::HumanFemale.apply(&mut settings);
        assert!(!settings.hide_player);
        assert_eq!((settings.player_body, settings.player_mount_body), (HUMAN_FEMALE_BODY, 0));
        assert_eq!(PlayerLook::of(&world(false, 9, 0)), None);
        assert_eq!(PlayerLook::of(&world(false, HUMAN_FEMALE_BODY, HORSE_BODY)), None);
        assert_eq!(PlayerLook::of(&world(false, 0, HORSE_BODY)), Some(PlayerLook::Marker));
    }

    /// The slider's levels are the server's, dimming as the clients do: none by day, darker every level, the
    /// Classic client by steps of 1/32 and the others by 1/31, and nothing past the darkest level the client applies.
    #[test]
    fn light_levels_map_onto_the_clients_dimming() {
        for classic_rule in [true, false] {
            assert_eq!(scene_light_factor(CLIENT_DAY_LIGHT_LEVEL, classic_rule), 1.0);
            for level in 1..=MAX_CLIENT_LIGHT_LEVEL {
                let factor = scene_light_factor(level, classic_rule);
                assert!(factor > 0.0 && factor < scene_light_factor(level - 1, classic_rule), "{level}");
            }
        }
        assert_eq!(scene_light_factor(SERVER_NIGHT_LIGHT_LEVEL, true), 20.0 / 32.0);
        assert_eq!(scene_light_factor(SERVER_NIGHT_LIGHT_LEVEL, false), 1.0 - 12.0 / 31.0);
        assert_eq!(clamp_light_level(MAX_CLIENT_LIGHT_LEVEL + 1), MAX_CLIENT_LIGHT_LEVEL);
        assert_eq!(clamp_light_level(SERVER_DUNGEON_LIGHT_LEVEL), SERVER_DUNGEON_LIGHT_LEVEL);
    }

    #[test]
    fn player_z_stays_in_a_uo_position_and_moves_the_transform() {
        assert_eq!(clamp_player_z(-200), i8::MIN);
        assert_eq!(clamp_player_z(300), i8::MAX);
        assert_eq!(clamp_player_z(-20), -20);

        const Z: i8 = 40;
        let start = UOVec4::new(2700, 2100, 0, 0);
        let mut player = Player { current_pos: Some(start), prev_rendered_pos: Some(start) };
        let mut transform = Transform::from_translation(start.to_bevy_vec3_ignore_map());
        set_player_z(&mut player, &mut transform, Z);
        assert!(player.current_pos == Some(UOVec4 { z: Z, ..start }));
        assert_eq!(transform.translation, UOVec4 { z: Z, ..start }.to_bevy_vec3_ignore_map());
    }
}
