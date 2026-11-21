//! The control channel's typed, whitelisted runtime settings: the render style and time of day, the art and land
//! sources, and the named preferences and style settings the regression harness switches. Each command changes the
//! in-memory state exactly as its UI control does, so the systems that follow a UI change follow it too; there is
//! no generic setter, and a key outside `ControlSetting` is refused.
//!
//! Persistence is the UI's: the shader preset changes stay in memory (the shader window saves land.toml only when it
//! closes), and a change of `Settings` reaches the cache copies through the app's debounced save, as a preference
//! picked in the window does. The cache folder is the one of the process's working directory, the run's own copy in
//! a harness or preview run.
use crate::settings::{ClientTextureSource, LandTextureSource, Settings};
use crate::render::uniform_state::UniformState;
use crate::render::pipeline_readiness::WaitingRenderPipelines;
use crate::ui::dialogs::land_shader::switch_render_preset;
use crate::render::view::camera::{RenderZoom, MAX_ZOOM, MIN_ZOOM};
use crate::render::statics::static_source::StaticArtSourceState;
use crate::world::caves::{CaveSurroundings, CaveView};
use crate::render::mobiles::in_between::MobileInBetweenFrames;
use crate::world::mobile_motion::MobileStop;
use crate::render::lighting::light_cover::LightCover;
use crate::render::terrain::mesh_material::{LandShaderModePresets, PlayerFloorRule};
use crate::render::scene::RecomputeVisibleChunksEvent;
use crate::render::assets::land::cache::LandTextureCache;
use bevy::{ecs::system::SystemParam, prelude::*};
use serde::Deserialize;
use serde_json::{json, Value};

/// One whitelisted setting with its typed value, as `{"key": ..., "value": ...}`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(tag = "key", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ControlSetting {
    /// The player: core.toml `[core.world]`.
    HidePlayer(bool),
    PlayerBody(u16),
    PlayerMountBody(u16),
    /// Preferences, world: world_rendering.toml.
    ShowOnlyPlayerFloor(bool),
    PlayerFloorBuildingOnly(bool),
    Caves(CaveView),
    CaveSurroundings(CaveSurroundings),
    MobileInBetweenFrames(MobileInBetweenFrames),
    MobileStop(MobileStop),
    EnableStatics(bool),
    EnableStaticLights(bool),
    ShowHiddenMultiParts(bool),
    /// Preferences, texture: graphics.toml.
    CcTerrainTransitions(bool),
    EcTerrainTransitions(bool),
    /// The active style's settings in the shader window: land.toml `[<style>]`.
    LightCover(LightCover),
    PlayerFloorRule(PlayerFloorRule),
    LandTextureVariation(bool),
    /// The camera zoom of the mouse wheel and the +/- keys.
    Zoom(f32),
}

impl ControlSetting {
    /// Read `key` and `value`: an unknown key, or a value of the wrong type or out of range, is an error naming the
    /// allowed keys or the expected type.
    pub(super) fn parse(key: &str, value: Value) -> Result<Self, String> {
        let setting: Self = serde_json::from_value(json!({"key": key, "value": value}))
            .map_err(|error| format!("set: {error}"))?;
        if let Self::Zoom(zoom) = setting
            && !(zoom.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(&zoom)) {
            return Err(format!("set: zoom {zoom} is outside {MIN_ZOOM}..={MAX_ZOOM}"));
        }
        Ok(setting)
    }
}

/// The state the settings commands change and `status` reports.
#[derive(SystemParam)]
pub(super) struct ControlSettings<'w> {
    settings: Option<ResMut<'w, Settings>>,
    uniforms: Option<ResMut<'w, UniformState>>,
    presets: Option<ResMut<'w, LandShaderModePresets>>,
    zoom: ResMut<'w, RenderZoom>,
    land_cache: Option<Res<'w, LandTextureCache>>,
    art_source: Option<Res<'w, StaticArtSourceState>>,
    waiting_pipelines: Option<Res<'w, WaitingRenderPipelines>>,
}

impl ControlSettings<'_> {
    pub(super) fn settings(&self) -> Option<&Settings> {
        self.settings.as_deref()
    }

    pub(super) fn uniforms(&self) -> Option<&UniformState> {
        self.uniforms.as_deref()
    }

    /// The Render Style and Time of Day buttons of the shader window. Returns whether the active preset changed.
    pub(super) fn set_preset(&mut self, style: &str, time: &str) -> Result<bool, String> {
        let (Some(uniforms), Some(presets)) = (self.uniforms.as_mut(), self.presets.as_mut()) else {
            return Err("the shader presets are not loaded yet".into());
        };
        let before = uniforms.active_preset.clone();
        if before == format!("{style}.{time}") {
            return Ok(false);
        }
        switch_render_preset(uniforms, presets, style, time)?;
        Ok(true)
    }

    /// The Art and Land Texture Source choices of the preferences. Returns whether either changed.
    pub(super) fn set_sources(&mut self, art: ClientTextureSource, land: LandTextureSource) -> Result<bool, String> {
        let settings = self.settings.as_mut().ok_or("the settings are not loaded yet")?;
        let graphics = &settings.graphics;
        if graphics.art_texture_source == art && graphics.land_texture_source == land {
            return Ok(false);
        }
        let graphics = &mut settings.graphics;
        graphics.art_texture_source = art;
        graphics.land_texture_source = land;
        Ok(true)
    }

    /// One whitelisted setting, as its control sets it. Returns whether it changed.
    pub(super) fn set(&mut self, setting: ControlSetting,
        recompute: &mut MessageWriter<RecomputeVisibleChunksEvent>) -> Result<bool, String> {
        use ControlSetting as S;
        match setting {
            S::HidePlayer(_) | S::PlayerBody(_) | S::PlayerMountBody(_)
            | S::ShowOnlyPlayerFloor(_) | S::PlayerFloorBuildingOnly(_) | S::Caves(_) | S::CaveSurroundings(_) | S::EnableStatics(_)
            | S::MobileInBetweenFrames(_) | S::MobileStop(_) | S::EnableStaticLights(_) | S::ShowHiddenMultiParts(_) | S::CcTerrainTransitions(_)
            | S::EcTerrainTransitions(_) => {
                let settings = self.settings.as_mut().ok_or("the settings are not loaded yet")?;
                // Compared through a shared borrow first: a mutable one marks Settings changed even when equal.
                if !Self::differs(settings, setting) {
                    return Ok(false);
                }
                let settings = &mut **settings;
                let (world, graphics, player) =
                    (&mut settings.world_rendering, &mut settings.graphics, &mut settings.core.world);
                match setting {
                    S::HidePlayer(value) => player.hide_player = value,
                    S::PlayerBody(value) => player.player_body = value,
                    S::PlayerMountBody(value) => player.player_mount_body = value,
                    S::ShowOnlyPlayerFloor(value) => world.show_only_player_floor = value,
                    S::PlayerFloorBuildingOnly(value) => world.player_floor_building_only = value,
                    S::Caves(value) => world.caves = value,
                    S::CaveSurroundings(value) => world.cave_surroundings = value,
                    S::MobileInBetweenFrames(value) => world.mobile_in_between_frames = value,
                    S::MobileStop(value) => world.mobile_stop = value,
                    S::EnableStatics(value) => world.enable_statics = value,
                    S::EnableStaticLights(value) => world.enable_static_lights = value,
                    S::ShowHiddenMultiParts(value) => world.show_hidden_multi_parts = value,
                    S::CcTerrainTransitions(value) => graphics.cc_terrain_transitions = value,
                    S::EcTerrainTransitions(value) => graphics.ec_terrain_transitions = value,
                    _ => unreachable!("matched above"),
                }
                Ok(true)
            }
            S::LightCover(_) | S::PlayerFloorRule(_) | S::LandTextureVariation(_) => {
                let uniforms = self.uniforms.as_mut().ok_or("the shader presets are not loaded yet")?;
                let style = uniforms.active_preset.split_once('.').map_or("", |(style, _)| style).to_owned();
                // The shader window shows player_floor_rule and land_texture_variation only in the styles that have
                // them, and an edit marks the uniforms dirty.
                let changed = match setting {
                    S::LightCover(value) => uniforms.light_cover != value,
                    S::PlayerFloorRule(value) => match uniforms.player_floor_rule {
                        Some(rule) => rule != value,
                        None => return Err(format!("set: the style `{style}` has no player_floor_rule")),
                    },
                    S::LandTextureVariation(value) => match uniforms.land_texture_variation {
                        Some(variation) => variation != value,
                        None => return Err(format!("set: the style `{style}` has no land_texture_variation")),
                    },
                    _ => unreachable!("matched above"),
                };
                if changed {
                    match setting {
                        S::LightCover(value) => uniforms.light_cover = value,
                        S::PlayerFloorRule(value) => uniforms.player_floor_rule = Some(value),
                        S::LandTextureVariation(value) => uniforms.land_texture_variation = Some(value),
                        _ => unreachable!("matched above"),
                    }
                    uniforms.dirty = true;
                }
                Ok(changed)
            }
            S::Zoom(value) => {
                let window = self.settings.as_ref().map(|settings| &settings.app.window);
                // The wheel also dollies a free perspective camera along its view; the channel does not move cameras.
                if window.is_some_and(|window| window.free_camera && window.perspective_camera) {
                    return Err("set: zoom is not supported with the free perspective camera".into());
                }
                if self.zoom.0 == value {
                    return Ok(false);
                }
                self.zoom.write_val(value);
                recompute.write(RecomputeVisibleChunksEvent {});
                Ok(true)
            }
        }
    }

    fn differs(settings: &Settings, setting: ControlSetting) -> bool {
        let (world, graphics, player) = (&settings.world_rendering, &settings.graphics, &settings.core.world);
        match setting {
            ControlSetting::HidePlayer(value) => player.hide_player != value,
            ControlSetting::PlayerBody(value) => player.player_body != value,
            ControlSetting::PlayerMountBody(value) => player.player_mount_body != value,
            ControlSetting::ShowOnlyPlayerFloor(value) => world.show_only_player_floor != value,
            ControlSetting::PlayerFloorBuildingOnly(value) => world.player_floor_building_only != value,
            ControlSetting::Caves(value) => world.caves != value,
            ControlSetting::CaveSurroundings(value) => world.cave_surroundings != value,
            ControlSetting::MobileInBetweenFrames(value) => world.mobile_in_between_frames != value,
            ControlSetting::MobileStop(value) => world.mobile_stop != value,
            ControlSetting::EnableStatics(value) => world.enable_statics != value,
            ControlSetting::EnableStaticLights(value) => world.enable_static_lights != value,
            ControlSetting::ShowHiddenMultiParts(value) => world.show_hidden_multi_parts != value,
            ControlSetting::CcTerrainTransitions(value) => graphics.cc_terrain_transitions != value,
            ControlSetting::EcTerrainTransitions(value) => graphics.ec_terrain_transitions != value,
            _ => true,
        }
    }

    /// The applied values, for `status`: what the settings ask for and what the land cache and the static art
    /// collector actually switched to.
    pub(super) fn status(&self) -> Value {
        let settings = self.settings.as_deref().map(|settings| {
            let (world, graphics, window) = (&settings.world_rendering, &settings.graphics, &settings.app.window);
            let player = &settings.core.world;
            json!({
                "hide_player": player.hide_player, "player_body": player.player_body,
                "player_mount_body": player.player_mount_body,
                "art_texture_source": graphics.art_texture_source, "land_texture_source": graphics.land_texture_source,
                "show_only_player_floor": world.show_only_player_floor,
                "player_floor_building_only": world.player_floor_building_only, "caves": world.caves,
                "cave_surroundings": world.cave_surroundings, "enable_statics": world.enable_statics,
                "mobile_in_between_frames": world.mobile_in_between_frames, "mobile_stop": world.mobile_stop,
                "enable_static_lights": world.enable_static_lights,
                "show_hidden_multi_parts": world.show_hidden_multi_parts,
                "cc_terrain_transitions": graphics.cc_terrain_transitions,
                "ec_terrain_transitions": graphics.ec_terrain_transitions,
                "perspective_camera": window.perspective_camera, "free_camera": window.free_camera,
            })
        });
        json!({
            "settings": settings,
            "zoom": self.zoom.0,
            "applied_sources": {
                "land": self.land_cache.as_ref().map(|cache| cache.preferred_source()),
                "art": self.art_source.as_ref().and_then(|state| state.active_source),
            },
            "waiting_pipelines": self.waiting_pipelines.as_ref().map(|waiting| waiting.count()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_parse_typed_values_and_refuse_unknown_keys_types_and_ranges() {
        assert_eq!(ControlSetting::parse("show_only_player_floor", json!(true)).unwrap(),
            ControlSetting::ShowOnlyPlayerFloor(true));
        assert_eq!(ControlSetting::parse("player_floor_building_only", json!(true)).unwrap(),
            ControlSetting::PlayerFloorBuildingOnly(true));
        assert_eq!(ControlSetting::parse("caves", json!("drawn_over_map")).unwrap(),
            ControlSetting::Caves(CaveView::DrawnOverMap));
        assert_eq!(ControlSetting::parse("cave_surroundings", json!("black")).unwrap(),
            ControlSetting::CaveSurroundings(CaveSurroundings::Black));
        assert_eq!(ControlSetting::parse("mobile_in_between_frames", json!("off")).unwrap(),
            ControlSetting::MobileInBetweenFrames(MobileInBetweenFrames::Off));
        assert_eq!(ControlSetting::parse("mobile_in_between_frames", json!("rife_x4")).unwrap(),
            ControlSetting::MobileInBetweenFrames(MobileInBetweenFrames::RifeX4));
        assert_eq!(ControlSetting::parse("mobile_stop", json!("at_once")).unwrap(),
            ControlSetting::MobileStop(MobileStop::AtOnce));
        assert_eq!(ControlSetting::parse("light_cover", json!("clients")).unwrap(),
            ControlSetting::LightCover(LightCover::Clients));
        assert_eq!(ControlSetting::parse("player_floor_rule", json!("kr")).unwrap(),
            ControlSetting::PlayerFloorRule(PlayerFloorRule::Kr));
        assert_eq!(ControlSetting::parse("zoom", json!(1.5)).unwrap(), ControlSetting::Zoom(1.5));

        let unknown = ControlSetting::parse("udd_path", json!("/tmp")).unwrap_err();
        assert!(unknown.contains("unknown variant `udd_path`") && unknown.contains("show_only_player_floor"),
            "{unknown}");
        for (key, value) in [("enable_statics", json!("yes")), ("caves", json!("everywhere")),
            ("light_cover", json!(1)), ("zoom", json!("near")), ("zoom", json!(MIN_ZOOM / 2.0)),
            ("zoom", json!(MAX_ZOOM * 2.0))] {
            assert!(ControlSetting::parse(key, value.clone()).is_err(), "{key} = {value}");
        }
    }
}
