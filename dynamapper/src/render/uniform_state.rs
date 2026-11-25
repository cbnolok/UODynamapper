//! The live shader uniforms: what the land, art, lighting and post-process passes read this frame, with the active
//! preset they came from and the snapshot Undo restores. The shader presets (`settings::shader_presets`) build it at
//! startup and save from it; the Shading window edits it.
use crate::render::lighting::light_cover::{LightCover, LightCoverRule};
use crate::render::terrain::mesh_material::{
    local_light_intensity, ClientLocalLights, ClientNightLighting, GlobalLightingUniforms, LandEffectsUniform,
    LandLightingUniforms, PlayerFloorRule, SunAngles,
};
use crate::settings::shader_presets::{
    preset_group_draws_light_sprites, preset_group_follows_kr_light_rules, preset_group_has_night_map,
    preset_group_has_scene_multiplier, preset_key_group,
};
use bevy::prelude::*;

// Holds current values, a dirty flag, a saved snapshot for undo, and the active
// preset name (e.g. "kr.morning") so we know which mode × time to save into.
#[derive(Resource, Clone)]
pub struct UniformState {
    pub effects: LandEffectsUniform, // texture/rendering pipeline controls
    pub lighting: GlobalLightingUniforms, // global: grading, fog, gloom, tonemap, colors
    pub land_lighting: LandLightingUniforms, // land-specific: fill, rim, bent, diffuse intensities
    pub global_lighting: f32,        // scene-wide brightness scaler (maps to scene.global_lighting);
                                     // CLIENT_LIT_SCENE_MULTIPLIER in KR-like and Enhanced, which have none
    /// The light level and the night composite that dims by it, in every style (`None` only for a slot that lacks it,
    /// which the loads fill or refuse).
    pub client_lighting: Option<ClientNightLighting>,
    /// The KR client's static shadows on or off: `Some` in KR-like only.
    pub client_shadows: Option<bool>,
    /// The local lights of the active preset's style, not of its slot: `Some` in KR-like, Enhanced and UOD-Painterly.
    /// `effects.local_light_intensity` follows them.
    pub local_lights: Option<ClientLocalLights>,
    /// The land texture variation of the active preset's style, not of its slot: `Some` in KR-like, Enhanced and
    /// UOD-Painterly. `effects.land_texture_variation` follows it.
    pub land_texture_variation: Option<bool>,
    /// The rule that hides covered lights, of the active preset's style.
    pub light_cover: LightCover,
    /// The rule "Show Only the Player's Floor" follows, of the active preset's style: `Some` in KR-like and
    /// UOD-Painterly only.
    pub player_floor_rule: Option<PlayerFloorRule>,
    pub sun: SunAngles,              // static light direction (maps to scene.light_direction)
    pub dirty: bool,                 // when true, push to GPU materials this frame
    /// Snapshot of the last values saved to disk (for undo). `None` until first save.
    pub saved_snapshot: Option<UniformSnapshot>,
    /// Active preset key, e.g. "kr.morning" — used to decide which slot to save into
    /// and to record which preset to restore on next launch.
    pub active_preset: String,
}

/// A lightweight snapshot of the values we persist (no dirty flag needed).
#[derive(Clone, Copy)]
pub struct UniformSnapshot {
    pub effects: LandEffectsUniform,
    pub lighting: GlobalLightingUniforms,
    pub land_lighting: LandLightingUniforms,
    pub global_lighting: f32,
    pub client_lighting: Option<ClientNightLighting>,
    pub client_shadows: Option<bool>,
    pub local_lights: Option<ClientLocalLights>,
    pub land_texture_variation: Option<bool>,
    pub light_cover: LightCover,
    pub player_floor_rule: Option<PlayerFloorRule>,
    pub sun: SunAngles,
}

impl UniformState {
    /// Take a snapshot of the current live values.
    pub fn snapshot(&self) -> UniformSnapshot {
        UniformSnapshot {
            effects: self.effects,
            lighting: self.lighting,
            land_lighting: self.land_lighting,
            global_lighting: self.global_lighting,
            client_lighting: self.client_lighting,
            client_shadows: self.client_shadows,
            local_lights: self.local_lights,
            land_texture_variation: self.land_texture_variation,
            light_cover: self.light_cover,
            player_floor_rule: self.player_floor_rule,
            sun: self.sun,
        }
    }

    /// Restore from a snapshot (used by Undo).
    pub fn restore(&mut self, snap: &UniformSnapshot) {
        self.effects = snap.effects;
        self.lighting = snap.lighting;
        self.land_lighting = snap.land_lighting;
        self.global_lighting = snap.global_lighting;
        self.client_lighting = snap.client_lighting;
        self.client_shadows = snap.client_shadows;
        self.local_lights = snap.local_lights;
        self.land_texture_variation = snap.land_texture_variation;
        self.light_cover = snap.light_cover;
        self.player_floor_rule = snap.player_floor_rule;
        self.sun = snap.sun;
        self.dirty = true;
    }

    /// The rule the active style's `light_cover` gives: "clients" follows the KR client in KR-like and UOD-Painterly
    /// and the Enhanced client in the other styles.
    pub fn light_cover_rule(&self) -> LightCoverRule {
        let kr_rules = preset_key_group(&self.active_preset).is_some_and(preset_group_follows_kr_light_rules);
        LightCoverRule::of_style(self.light_cover, kr_rules)
    }

    /// Whether the active style draws the clients' light sprites rather than light.mul's lights.
    pub fn draws_light_sprites(&self) -> bool {
        preset_key_group(&self.active_preset).is_some_and(preset_group_draws_light_sprites)
    }

    /// Whether the active style dims by the Classic client's rule rather than the KR and Enhanced clients' composite.
    pub fn dims_by_classic_rule(&self) -> bool {
        preset_key_group(&self.active_preset).is_some_and(|group| !preset_group_has_night_map(group))
    }

    /// Whether the active style keeps a scene multiplier of its own (`global_lighting`).
    pub fn has_scene_multiplier(&self) -> bool {
        preset_key_group(&self.active_preset).is_some_and(preset_group_has_scene_multiplier)
    }

    /// The rule "Show Only the Player's Floor" follows in the active style: its `player_floor_rule`, ClassicUO's in
    /// the styles without one.
    pub fn effective_player_floor_rule(&self) -> PlayerFloorRule {
        self.player_floor_rule.unwrap_or(PlayerFloorRule::ClassicUo)
    }

    /// Bring `effects.local_light_intensity` in line with `local_lights`.
    pub fn sync_local_light_intensity(&mut self) {
        self.effects.local_light_intensity = local_light_intensity(self.local_lights.as_ref());
    }

    /// The `effects.land_texture_variation` flag `land_texture_variation` gives: on only where the style has it on.
    pub fn land_texture_variation_flag(&self) -> u32 {
        u32::from(self.land_texture_variation == Some(true))
    }

    /// Bring `effects.land_texture_variation` in line with `land_texture_variation`.
    pub fn sync_land_texture_variation(&mut self) {
        self.effects.land_texture_variation = self.land_texture_variation_flag();
    }
}
