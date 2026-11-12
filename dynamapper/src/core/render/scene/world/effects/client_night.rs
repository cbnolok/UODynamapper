// world::effects::client_night — the KR and Enhanced clients' night lighting composite.
//
// Both clients darken and tint the finished frame in one LDR pass (UOKR.exe
// `UOLightingPostProcessEffect::BuildCompositeQuad` 0x0041f5c0 and `UOLightingShader::SetupTextureStages`
// 0x005441c0; the Enhanced client runs the same code). Per channel, on the 8-bit (sRGB-encoded) values:
//
//     out = clamp(4 * (c * level + B * (1 - level)) * scene, 0, 1)
//
// - c = 0.25 + 0.2 * (gamma - 0.5): the ambient term, 0.25 at the client's default gamma 0.5.
// - level = 1 - L / 31 for the server light level L (0 = full day, where the composite is the identity).
// - B, the light buffer: the night map (TerrainTexture.uop entry 21, packed in tex_aux_ec.uddp) with the light
//   sprites lerped over it. B is weighted by 4 * (1 - level), so lights and the night tint vanish by day.
//
// The night map (`UONightMapShader::SetupTextureStages` 0x00544600) repeats every 26 world units on the camera
// plane, with square pixels, rotated 45 degrees and anchored to the camera focus P:
//
//     u = (s_x + P_r) / 26,  v = (s_y + P_d) / 26,  bilinear, wrapping
//
// where s is the pixel's offset from the screen centre in world units (s_y down) and, in our axes (x = UO x,
// z = UO y), P_r = (x - z) / sqrt(2) is the focus along screen right and P_d = (x + z) / sqrt(2) along screen
// down. The camera translation stands in for P: along screen right it differs from the focus by nothing, along
// screen down by a constant, which only shifts the pattern.
//
// Dynamapper's controls change the night map N only (the lights keep their colours):
//
//     T  = N * overlay_color / NIGHT_MAP_MEAN
//     N' = clamp(luma(T) * (1 - overlay_strength) + T * overlay_strength, 0, 1)
//
// The defaults (strength 1, colour = the map's mean) give N' = N: the client. Strength 0 replaces the map by its
// luminance; the composite is affine in B, so the night then dims by the luminance of the client's multiplier,
// without the tint. Another colour rescales the map's channels, keeping its cloud pattern.
//
// Lights: until the packages carry the client's light sprites (tileart light scale and offsets), the static light
// quads of `art::static_lights` stand in for them. In KR-like and Enhanced they are drawn on `LIGHT_BUFFER_LAYER`
// by an offscreen camera into the light buffer image, with straight alpha blending from a transparent clear. That
// leaves L, the lights lerped over black with L.a their coverage, so B = L.rgb + N' * (1 - L.a) is the client's
// sequential lerp of the sprites over the night map.
//
// KR-like's static shadows (the KR client's shadow effect, research P3.4) composite in the same pass, first, as the
// client runs its shadow effect before its lighting effect. The shadow quads of `art::client_shadow.wesl` leave the
// shadow buffer S in the frame's alpha (1 outside shadows, times 1 - a per shadow texel), and per channel on the
// 8-bit values:
//
//     scene' = scene * clamp(S + 1 - s, 0, 1),  s = CLIENT_SHADOW_INTENSITY = 0.25
//
// so a shadow darkens by at most 0.25 and overlapping shadows do not darken further. The pass writes alpha 1.
//
// The composite runs after the fog pass (and tonemapping) and before FXAA and SMAA, on the PlayerCamera only, and
// only while the active preset has `client_lighting` (KR-like and Enhanced) and darkens the frame: a light level
// above 0, or KR-like's client shadows. Classic and UOD-Painterly keep `global_lighting` and never run it.

use crate::configs::shader_presets::UniformState;
use crate::core::render::scene::camera::{PlayerCamera, RenderZoom, ORTHO_SIZE_FACTOR};
use crate::core::render::scene::world::effects::fog_pass::FogPassSystems;
use crate::core::render::scene::world::land::mesh_material::ClientNightLighting;
use crate::core::system_sets::StartupSysSet;
use crate::core::uo_files_loader::TexAuxEcPackageRes;
use crate::prelude::*;
use bevy::anti_alias::{fxaa::fxaa, smaa::smaa};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraUpdateSystems, Hdr, ImageRenderTarget, RenderTarget};
use bevy::core_pipeline::tonemapping::{tonemapping, DebandDither, Tonemapping};
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::ecs::error::BevyError;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec2;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    binding_types::{sampler, texture_2d, uniform_buffer},
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, Canonical,
    ColorTargetState, ColorWrites, Extent3d, FragmentState, Operations, PipelineCache, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
    Specializer, SpecializerKey, TextureDimension, TextureFormat, TextureSampleType, Variants,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{FallbackImageZero, GpuImage};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use udd_assets::tex_aux_ec::EC_NIGHT_MAP_TERRAIN_TEXTURE_ENTRY;

// ---- The client's constants --------------------------------------------------

/// `level = 1 - L / CLIENT_LIGHT_LEVEL_DIVISOR`: `UOAtmosphere::OnGlobalLightLevel` 0x0056e8a0 multiplies the level by
/// 0.032258064 = 1 / 31.
pub const CLIENT_LIGHT_LEVEL_DIVISOR: f32 = 31.0;
/// The darkest light level the client applies: it takes server levels below 31.
pub const MAX_CLIENT_LIGHT_LEVEL: u32 = 30;
/// Full day: the composite is the identity, so neither it nor the light buffer camera runs.
pub const CLIENT_DAY_LIGHT_LEVEL: u32 = 0;
/// The composite's ambient term c = 0.25 + 0.2 * (gamma - 0.5) (floats 0x00b86fac, 0x00b86f94, 0x00b4e048), at the
/// client's default gamma 0.5 (defaults routine 0x0047b660).
pub const CLIENT_AMBIENT_TERM: f32 = 0.25;
/// The composite's second texture stage, MODULATE4X.
pub const CLIENT_COMPOSITE_GAIN: f32 = 4.0;
/// The composite's `level` at full day: the identity.
pub const CLIENT_FULL_DAY_LEVEL: f32 = 1.0;
/// Mean colour of the night map, TerrainTexture.uop entry 21 as packed losslessly in tex_aux_ec.uddp (byte-identical
/// in the KR and Enhanced clients): channel sums R 210631, G 288859, B 2570545 over its 256 x 256 texels, divided by
/// 65536 * 255. The research table's (0.012, 0.016, 0.152) came from another DXT decoder.
pub const NIGHT_MAP_MEAN: Vec3 = Vec3::new(0.012604, 0.017285, 0.153818);
/// A loaded night map whose mean differs from `NIGHT_MAP_MEAN` by more than one 8-bit step in a channel no longer
/// matches the overlay colour's default.
const NIGHT_MAP_MEAN_TOLERANCE: f32 = 1.0 / 255.0;
/// One repeat of the night map spans 26 world units: twice the client's frustum half-width 13 (`UOWorldViewer`
/// +0x6c, 0x00b4f2e4), at every zoom.
pub const NIGHT_MAP_WORLD_UNITS_PER_REPEAT: f64 = 26.0;
/// The shadow composite's intensity s: `InitPostProcessEffects` stores 0.25 (0x00c4a024) over the constructor's 0.4
/// (0x0043f740); only debug keys change it.
pub const CLIENT_SHADOW_INTENSITY: f32 = 0.25;
/// The shadow composite's s while KR-like's client shadows are off, and in the other styles: no shadow.
pub const NO_CLIENT_SHADOW: f32 = 0.0;
/// The shadow buffer outside every shadow (the client clears S to white).
pub const CLIENT_SHADOW_BUFFER_CLEAR: f32 = 1.0;

// ---- Dynamapper's controls ------------------------------------------------------

/// Night overlay strength bounds: 0 removes the tint, 1 is the client, 2 doubles the tint around the luminance.
pub const MIN_NIGHT_OVERLAY_STRENGTH: f32 = 0.0;
pub const MAX_NIGHT_OVERLAY_STRENGTH: f32 = 2.0;
/// The luminance the night overlay keeps at strength 0: Rec. 709 weights on the 8-bit values.
pub const NIGHT_OVERLAY_LUMA_WEIGHTS: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

// ---- Rendering --------------------------------------------------------------------

/// The render layer of the light sprites the light buffer camera draws.
pub const LIGHT_BUFFER_LAYER: usize = 1;
/// The light buffer camera renders before the world camera (order 0), which reads its image in the composite.
const LIGHT_BUFFER_CAMERA_ORDER: isize = -1;
/// A float buffer: the lerps then stay exact on the 8-bit values the light textures carry (an sRGB 8-bit target
/// would blend them in linear light).
const LIGHT_BUFFER_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// The light buffer's size before the world camera reports its target.
const INITIAL_LIGHT_BUFFER_SIDE: u32 = 1;
/// The flat night map that stands in when the package has none: one texel.
const FLAT_NIGHT_MAP_SIDE: u32 = 1;
const RGBA_CHANNELS: usize = 4;
const BYTE_MAX: f32 = 255.0;
const OPAQUE_ALPHA: u8 = u8::MAX;
const COMPOSITE_SHADER: &str = "shaders/world/effects/client_night_composite.wesl";

// ---- The composite's maths (mirrored by client_night_composite.wesl) ---------------

/// The client's `level` for a light level: 1 at full day, falling by 1/31 per level.
pub fn client_light_level_weight(light_level: u32) -> f32 {
    CLIENT_FULL_DAY_LEVEL - light_level.min(MAX_CLIENT_LIGHT_LEVEL) as f32 / CLIENT_LIGHT_LEVEL_DIVISOR
}

/// The night map texel after Dynamapper's strength and colour (see the module notes).
pub fn night_overlay(night: Vec3, strength: f32, color_ratio: Vec3) -> Vec3 {
    let tinted = night * color_ratio;
    let luma = Vec3::splat(tinted.dot(NIGHT_OVERLAY_LUMA_WEIGHTS));
    (luma * (1.0 - strength) + tinted * strength).clamp(Vec3::ZERO, Vec3::ONE)
}

/// B: the lights (lerped over black, alpha = coverage) over the night overlay.
pub fn light_buffer_over_night(lights: Vec4, night: Vec3) -> Vec3 {
    lights.truncate() + night * (1.0 - lights.w)
}

/// The KR client's shadow composite's factor for a shadow buffer value: `clamp(S + 1 - s)`.
pub fn client_shadow_factor(shadow_buffer: f32, intensity: f32) -> f32 {
    (shadow_buffer + CLIENT_SHADOW_BUFFER_CLEAR - intensity).clamp(0.0, 1.0)
}

/// The client composite on 8-bit (sRGB-encoded) values.
pub fn client_composite(scene: Vec3, light_buffer: Vec3, level: f32) -> Vec3 {
    let multiplier = CLIENT_COMPOSITE_GAIN * (Vec3::splat(CLIENT_AMBIENT_TERM * level) + light_buffer * (1.0 - level));
    (multiplier * scene).clamp(Vec3::ZERO, Vec3::ONE)
}

/// Where the night map sits: xy = half the view in map repeats (x across, y down, both with the horizontal world
/// units per pixel), zw = the focus along screen right and down in repeats, modulo one repeat so f32 keeps it exact.
pub fn night_map_placement(focus_xz: DVec2, half_view_world: Vec2) -> Vec4 {
    let along_right = (focus_xz.x - focus_xz.y) * std::f64::consts::FRAC_1_SQRT_2;
    let along_down = (focus_xz.x + focus_xz.y) * std::f64::consts::FRAC_1_SQRT_2;
    let repeat = NIGHT_MAP_WORLD_UNITS_PER_REPEAT;
    Vec4::new(
        (f64::from(half_view_world.x) / repeat) as f32,
        (f64::from(half_view_world.y) / repeat) as f32,
        (along_right.rem_euclid(repeat) / repeat) as f32,
        (along_down.rem_euclid(repeat) / repeat) as f32,
    )
}

/// The night map coordinate of a screen uv (0..1, y down) for a placement from `night_map_placement`.
pub fn night_map_uv(screen_uv: Vec2, placement: Vec4) -> Vec2 {
    (screen_uv * 2.0 - Vec2::ONE) * placement.xy() + placement.zw()
}

// ---- Components and resources ------------------------------------------------------

/// The composite's parameters, on the PlayerCamera while client lighting is active.
///
/// Must match `ClientNightCompositeUniform` in `client_night_composite.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug, PartialEq)]
#[extract_app(bevy::render::RenderApp)]
pub struct ClientNightCompositeUniform {
    /// The client's `level`, `client_light_level_weight`.
    pub level: f32,
    pub overlay_strength: f32,
    /// 1 when the night map is bound, 0 when the package lacks it and `flat_night_color` stands in.
    pub night_map_present: u32,
    /// The shadow composite's s: `CLIENT_SHADOW_INTENSITY` while KR-like's client shadows are on, else 0.
    pub shadow_intensity: f32,
    /// The overlay colour over `NIGHT_MAP_MEAN`, per channel (w unused).
    pub overlay_color_ratio: Vec4,
    /// `NIGHT_MAP_MEAN`, the night colour without a map (w unused).
    pub flat_night_color: Vec4,
    /// `night_map_placement`.
    pub night_uv: Vec4,
}

impl ClientNightCompositeUniform {
    pub fn new(lighting: &ClientNightLighting, shadow_intensity: f32, night_map_present: bool, night_uv: Vec4) -> Self {
        Self {
            level: client_light_level_weight(lighting.light_level),
            overlay_strength: lighting.overlay_strength,
            night_map_present: u32::from(night_map_present),
            shadow_intensity,
            overlay_color_ratio: (lighting.overlay_color / NIGHT_MAP_MEAN).extend(0.0),
            flat_night_color: NIGHT_MAP_MEAN.extend(0.0),
            night_uv,
        }
    }
}

/// The images the composite samples, next to its uniform on the PlayerCamera.
#[derive(Component, ExtractComponent, Clone)]
#[extract_app(bevy::render::RenderApp)]
pub struct ClientNightCompositeTextures {
    pub night_map: Handle<Image>,
    pub light_buffer: Handle<Image>,
}

/// The night map image, or a texel of its mean when the package lacks it.
#[derive(Resource)]
pub struct ClientNightMap {
    pub image: Handle<Image>,
    pub from_package: bool,
}

/// The image the light buffer camera draws the lights into, sized like the world camera's target.
#[derive(Resource)]
pub struct ClientLightBuffer {
    pub image: Handle<Image>,
}

/// The offscreen camera that draws `LIGHT_BUFFER_LAYER` into the light buffer, a child of the PlayerCamera.
/// Extracted so render-world passes meant for the world view can skip its view.
#[derive(Component, ExtractComponent, Clone, Copy)]
#[extract_app(bevy::render::RenderApp)]
pub struct LightBufferCamera;

// ---- Plugin ------------------------------------------------------------------------------

pub struct ClientNightLightingPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ClientNightLightingPlugin);

impl Plugin for ClientNightLightingPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            ExtractComponentPlugin::<ClientNightCompositeUniform>::default(),
            UniformComponentPlugin::<ClientNightCompositeUniform>::default(),
            ExtractComponentPlugin::<ClientNightCompositeTextures>::default(),
            ExtractComponentPlugin::<LightBufferCamera>::default(),
        ))
        .add_systems(
            Startup,
            sys_setup_client_night_resources
                .in_set(StartupSysSet::SetupSceneStage1)
                .after(StartupSysSet::LoadStartupUOFiles),
        )
        .add_systems(Update, sys_spawn_light_buffer_camera)
        .add_systems(
            PostUpdate,
            (
                sys_sync_light_buffer_camera.before(CameraUpdateSystems),
                sys_sync_client_night_composite.after(CameraUpdateSystems),
            ),
        );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_client_night_composite_pipeline)
            .add_systems(Render, prepare_client_night_composite_pipelines.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                client_night_composite
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping)
                    .after(FogPassSystems)
                    .before(fxaa)
                    .before(smaa),
            );
    }
}

// ---- Main world ------------------------------------------------------------------------

/// The active preset's client lighting when it darkens the frame (KR-like and Enhanced below full day).
fn night_lighting(uniform_state: &UniformState) -> Option<ClientNightLighting> {
    uniform_state.client_lighting.filter(|lighting| lighting.light_level != CLIENT_DAY_LIGHT_LEVEL)
}

/// The shadow composite's s for the active preset: KR-like's client shadows on or off.
fn client_shadow_intensity(uniform_state: &UniformState) -> f32 {
    if uniform_state.client_shadows == Some(true) { CLIENT_SHADOW_INTENSITY } else { NO_CLIENT_SHADOW }
}

/// The active preset's client lighting and shadow intensity when the composite changes the frame: below full day,
/// or with KR-like's client shadows.
fn composite_settings(uniform_state: &UniformState) -> Option<(ClientNightLighting, f32)> {
    let lighting = uniform_state.client_lighting?;
    let shadow_intensity = client_shadow_intensity(uniform_state);
    (lighting.light_level != CLIENT_DAY_LIGHT_LEVEL || shadow_intensity > NO_CLIENT_SHADOW)
        .then_some((lighting, shadow_intensity))
}

/// The mean colour of RGBA8 pixels, as 0..1 values.
pub fn rgba_mean(rgba: &[u8]) -> Vec3 {
    let pixels = rgba.chunks_exact(RGBA_CHANNELS);
    let count = pixels.len().max(1) as f64;
    let sums = pixels.fold([0u64; 3], |mut sums, pixel| {
        for (sum, channel) in sums.iter_mut().zip(pixel) {
            *sum += u64::from(*channel);
        }
        sums
    });
    let mean = |sum: u64| (sum as f64 / count / f64::from(BYTE_MAX)) as f32;
    Vec3::new(mean(sums[0]), mean(sums[1]), mean(sums[2]))
}

/// The night map as a wrapping, bilinear texture whose texels are the 8-bit values (no sRGB decode).
fn night_map_image(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

fn flat_night_map_rgba() -> Vec<u8> {
    let [r, g, b] = NIGHT_MAP_MEAN.to_array().map(|channel| (channel * BYTE_MAX).round() as u8);
    [r, g, b, OPAQUE_ALPHA].repeat((FLAT_NIGHT_MAP_SIDE * FLAT_NIGHT_MAP_SIDE) as usize)
}

fn sys_setup_client_night_resources(
    mut commands: Commands,
    tex_aux: Option<Res<TexAuxEcPackageRes>>,
    mut images: ResMut<Assets<Image>>,
) {
    log_system_add_startup::<ClientNightLightingPlugin>(StartupSysSet::SetupSceneStage1, fname!());
    let night_map = match tex_aux.as_deref() {
        None => Err("there is no tex_aux_ec.uddp".to_string()),
        Some(package) => package.0.load_terrain_texture_rgba(EC_NIGHT_MAP_TERRAIN_TEXTURE_ENTRY)
            .map_err(|error| format!("cannot read the night map from tex_aux_ec.uddp: {error:#}")),
    };
    let (image, from_package) = match night_map {
        Ok((width, height, rgba)) => {
            let mean = rgba_mean(&rgba);
            if !mean.abs_diff_eq(NIGHT_MAP_MEAN, NIGHT_MAP_MEAN_TOLERANCE) {
                log::warn!(
                    "The night map's mean colour {mean} is not NIGHT_MAP_MEAN {NIGHT_MAP_MEAN}: the night overlay \
                     colour's default no longer reproduces the client"
                );
            }
            (night_map_image(u32::from(width), u32::from(height), rgba), true)
        }
        Err(reason) => {
            log::warn!(
                "No night map ({reason}): KR-like and Enhanced nights use its flat mean colour {NIGHT_MAP_MEAN}"
            );
            (night_map_image(FLAT_NIGHT_MAP_SIDE, FLAT_NIGHT_MAP_SIDE, flat_night_map_rgba()), false)
        }
    };
    commands.insert_resource(ClientNightMap { image: images.add(image), from_package });

    let light_buffer =
        Image::new_target_texture(INITIAL_LIGHT_BUFFER_SIDE, INITIAL_LIGHT_BUFFER_SIDE, LIGHT_BUFFER_FORMAT, None);
    commands.insert_resource(ClientLightBuffer { image: images.add(light_buffer) });
}

/// Give each new PlayerCamera a light buffer camera: same pose (a child with the identity transform), same
/// projection (copied every frame), drawing only `LIGHT_BUFFER_LAYER`.
///
/// It is HDR on purpose, unlike the world camera (see camera.rs): its main texture is then a float texture, so the
/// alpha blending runs on the 8-bit values the light textures carry, as the client's does, and its only content is
/// unlit light quads, which neither the land shader's tonemapping nor Bevy's touches (Tonemapping::None).
fn sys_spawn_light_buffer_camera(
    mut commands: Commands,
    light_buffer: Option<Res<ClientLightBuffer>>,
    cameras: Query<(Entity, &Projection), Added<PlayerCamera>>,
) {
    let Some(light_buffer) = light_buffer else {
        return;
    };
    for (world_camera, projection) in &cameras {
        let light_camera = commands
            .spawn((
                LightBufferCamera,
                Camera3d::default(),
                Camera {
                    order: LIGHT_BUFFER_CAMERA_ORDER,
                    is_active: false,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                RenderTarget::Image(ImageRenderTarget { handle: light_buffer.image.clone(), scale_factor: 1.0 }),
                Hdr,
                Tonemapping::None,
                DebandDither::Disabled,
                Msaa::Off,
                RenderLayers::layer(LIGHT_BUFFER_LAYER),
                projection.clone(),
                Transform::IDENTITY,
            ))
            .id();
        commands.entity(world_camera).add_child(light_camera);
    }
}

/// Keep the light buffer camera active only while the composite runs, its projection equal to the world camera's
/// and its image the size of the world camera's target.
fn sys_sync_light_buffer_camera(
    uniform_state: Res<UniformState>,
    light_buffer: Option<Res<ClientLightBuffer>>,
    mut images: ResMut<Assets<Image>>,
    world_cameras: Query<(&Camera, Ref<Projection>), (With<PlayerCamera>, Without<LightBufferCamera>)>,
    mut light_cameras: Query<(&mut Camera, &mut Projection), With<LightBufferCamera>>,
) {
    let (Some(light_buffer), Ok((world_camera, world_projection)), Ok((mut light_camera, mut light_projection))) =
        (light_buffer, world_cameras.single(), light_cameras.single_mut())
    else {
        return;
    };
    let active = night_lighting(&uniform_state).is_some();
    let activated = active && !light_camera.is_active;
    if light_camera.is_active != active {
        light_camera.is_active = active;
    }
    if !active {
        return;
    }
    if activated || world_projection.is_changed() {
        *light_projection = (*world_projection).clone();
    }
    let Some(size) = world_camera.physical_target_size() else {
        return;
    };
    let extent = Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 };
    if images.get(&light_buffer.image).is_some_and(|image| image.texture_descriptor.size != extent)
        && let Some(mut image) = images.get_mut(&light_buffer.image)
    {
        image.resize(extent);
    }
}

/// Half the world camera's view in world units: x across; y down with the horizontal units per pixel (square pixels).
fn half_view_world(camera: &Camera, projection: &Projection, zoom: f32) -> Option<Vec2> {
    let physical = camera.physical_target_size()?.as_vec2();
    let half_width = match projection {
        Projection::Orthographic(orthographic) => orthographic.area.width() * 0.5,
        // The perspective camera frames like the orthographic one (camera/perspective.rs).
        _ => camera.logical_target_size()?.x / ORTHO_SIZE_FACTOR * zoom * 0.5,
    };
    (physical.x > 0.0).then(|| Vec2::new(half_width, half_width * physical.y / physical.x))
}

/// Put the composite on the PlayerCamera while the active preset's client lighting or shadows darken the frame, and
/// keep it current.
fn sys_sync_client_night_composite(
    mut commands: Commands,
    uniform_state: Res<UniformState>,
    night_map: Option<Res<ClientNightMap>>,
    light_buffer: Option<Res<ClientLightBuffer>>,
    zoom: Res<RenderZoom>,
    mut cameras: Query<
        (Entity, &Camera, &Projection, &Transform, Option<&mut ClientNightCompositeUniform>),
        With<PlayerCamera>,
    >,
) {
    let Ok((entity, camera, projection, transform, current)) = cameras.single_mut() else {
        return;
    };
    let (Some((lighting, shadow_intensity)), Some(night_map), Some(light_buffer)) =
        (composite_settings(&uniform_state), night_map, light_buffer)
    else {
        if current.is_some() {
            commands.entity(entity).remove::<(ClientNightCompositeUniform, ClientNightCompositeTextures)>();
        }
        return;
    };
    let Some(half_view) = half_view_world(camera, projection, zoom.0) else {
        return;
    };
    let focus = DVec2::new(f64::from(transform.translation.x), f64::from(transform.translation.z));
    let uniform = ClientNightCompositeUniform::new(
        &lighting,
        shadow_intensity,
        night_map.from_package,
        night_map_placement(focus, half_view),
    );
    match current {
        Some(mut current) => {
            if *current != uniform {
                *current = uniform;
            }
        }
        None => {
            commands.entity(entity).insert((
                uniform,
                ClientNightCompositeTextures {
                    night_map: night_map.image.clone(),
                    light_buffer: light_buffer.image.clone(),
                },
            ));
        }
    }
}

// ---- Render world --------------------------------------------------------------------------

#[derive(Resource)]
struct ClientNightCompositePipeline {
    layout: BindGroupLayoutDescriptor,
    variants: Variants<RenderPipeline, ClientNightCompositeSpecializer>,
}

struct ClientNightCompositeSpecializer;

#[derive(PartialEq, Eq, Hash, Clone, Copy, SpecializerKey)]
struct ClientNightCompositeKey {
    target_format: TextureFormat,
}

impl Specializer<RenderPipeline> for ClientNightCompositeSpecializer {
    type Key = ClientNightCompositeKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.fragment_mut()?.set_target(
            0,
            ColorTargetState { format: key.target_format, blend: None, write_mask: ColorWrites::ALL },
        );
        Ok(key)
    }
}

#[derive(Component)]
struct ClientNightCompositePipelineId(CachedRenderPipelineId);

fn init_client_night_composite_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "client_night_composite_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                // The frame, read texel by texel.
                texture_2d(TextureSampleType::Float { filterable: false }),
                uniform_buffer::<ClientNightCompositeUniform>(true),
                // The night map and its wrapping bilinear sampler.
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                // The light buffer, read texel by texel.
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let descriptor = RenderPipelineDescriptor {
        label: Some("client_night_composite_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: asset_server.load(COMPOSITE_SHADER),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    };
    commands.insert_resource(ClientNightCompositePipeline {
        layout,
        variants: Variants::new(ClientNightCompositeSpecializer, descriptor),
    });
}

fn prepare_client_night_composite_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<ClientNightCompositePipeline>,
    views: Query<(Entity, &ExtractedView, Has<ClientNightCompositeUniform>), With<ExtractedCamera>>,
) -> Result<(), BevyError> {
    for (entity, view, has_composite) in &views {
        if !has_composite {
            commands.entity(entity).remove::<ClientNightCompositePipelineId>();
            continue;
        }
        let key = ClientNightCompositeKey { target_format: view.target_format };
        let pipeline_id = pipeline.variants.specialize(&pipeline_cache, key)?;
        commands.entity(entity).insert(ClientNightCompositePipelineId(pipeline_id));
    }
    Ok(())
}

fn client_night_composite(
    view: ViewQuery<(
        &ViewTarget,
        &DynamicUniformIndex<ClientNightCompositeUniform>,
        &ClientNightCompositeTextures,
        &ClientNightCompositePipelineId,
    )>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<ClientNightCompositePipeline>,
    uniforms: Res<ComponentUniforms<ClientNightCompositeUniform>>,
    images: Res<RenderAssets<GpuImage>>,
    no_lights: Res<FallbackImageZero>,
    render_device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (view_target, uniform_index, textures, pipeline_id) = view.into_inner();
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(pipeline_id.0) else {
        return;
    };
    let Some(uniform_binding) = uniforms.uniforms().binding() else {
        return;
    };
    // The night map uploads with the first frames; until then the frame stays as drawn.
    let Some(night_map) = images.get(&textures.night_map) else {
        return;
    };
    // Before the light buffer exists on the GPU there are no lights: a transparent texel.
    let light_buffer = images.get(&textures.light_buffer).map_or(&no_lights.texture_view, |image| &image.texture_view);

    let post_process = view_target.post_process_write();
    let bind_group = render_device.create_bind_group(
        "client_night_composite_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            post_process.source,
            uniform_binding,
            &night_map.texture_view,
            &night_map.sampler,
            light_buffer,
        )),
    );
    let pass_descriptor = RenderPassDescriptor {
        label: Some("client_night_composite_pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post_process.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    };
    let mut render_pass = ctx.command_encoder().begin_render_pass(&pass_descriptor);
    render_pass.set_pipeline(render_pipeline);
    render_pass.set_bind_group(0, &bind_group, &[uniform_index.index()]);
    render_pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// light_generic (TerrainTexture.uop entry 1), the sprite of 462 of the client's 599 light sources: its colour and
    /// its peak alpha, 115.
    const CLIENT_GENERIC_LIGHT_COLOR: [u8; 3] = [192, 162, 93];
    const CLIENT_LIGHT_SPRITE_PEAK_ALPHA: f32 = 115.0 / 255.0;

    /// The research's mean of the night map (another DXT decoder), from which its table was computed.
    const RESEARCH_NIGHT_MAP_MEAN: Vec3 = Vec3::new(0.012, 0.016, 0.152);
    /// The research table rounds to three decimals, from a mean also given to three.
    const TABLE_TOLERANCE: f32 = 0.0015;
    /// Our measured mean moves the multipliers by up to this much from the table (the mean's blue is 0.0018 higher).
    const MEASURED_MEAN_TOLERANCE: f32 = 0.007;
    const FLAT_GREY: f32 = 0.5;
    const NO_LIGHTS: Vec4 = Vec4::ZERO;
    /// The research's scene multipliers at gamma 0.5 for the mean night map.
    const TABLE: [(u32, Vec3); 2] = [(10, Vec3::new(0.693, 0.699, 0.874)), (26, Vec3::new(0.201, 0.216, 0.671))];

    fn client_settings() -> ClientNightLighting {
        ClientNightLighting { light_level: 0, overlay_strength: 1.0, overlay_color: NIGHT_MAP_MEAN }
    }

    /// The composite of a flat grey scene over a night map of uniform colour `night`, divided by the grey: the scene
    /// multiplier.
    fn flat_grey_multiplier(settings: &ClientNightLighting, night: Vec3) -> Vec3 {
        let uniform = ClientNightCompositeUniform::new(settings, NO_CLIENT_SHADOW, true, Vec4::ZERO);
        let overlay = night_overlay(night, uniform.overlay_strength, uniform.overlay_color_ratio.truncate());
        let light_buffer = light_buffer_over_night(NO_LIGHTS, overlay);
        client_composite(Vec3::splat(FLAT_GREY), light_buffer, uniform.level) / FLAT_GREY
    }

    #[test]
    fn full_day_is_the_identity() {
        let uniform = ClientNightCompositeUniform::new(&client_settings(), NO_CLIENT_SHADOW, true, Vec4::ZERO);
        assert_eq!(uniform.level, CLIENT_FULL_DAY_LEVEL);
        for scene in [Vec3::ZERO, Vec3::new(0.1, 0.5, 0.9), Vec3::ONE] {
            let light_buffer = light_buffer_over_night(Vec4::new(0.3, 0.2, 0.1, 0.4), NIGHT_MAP_MEAN);
            assert_eq!(client_composite(scene, light_buffer, uniform.level), scene);
        }
    }

    #[test]
    fn default_overlay_is_the_night_map_unchanged() {
        let uniform = ClientNightCompositeUniform::new(&client_settings(), NO_CLIENT_SHADOW, true, Vec4::ZERO);
        assert_eq!(uniform.overlay_color_ratio.truncate(), Vec3::ONE);
        for texel in [NIGHT_MAP_MEAN, Vec3::new(0.0, 0.03, 0.259), Vec3::new(0.02, 0.01, 0.086)] {
            assert_eq!(night_overlay(texel, uniform.overlay_strength, Vec3::ONE), texel);
        }
    }

    #[test]
    fn flat_grey_matches_the_research_table() {
        for (light_level, expected) in TABLE {
            let settings = ClientNightLighting { light_level, ..client_settings() };
            let research = flat_grey_multiplier(&settings, RESEARCH_NIGHT_MAP_MEAN);
            assert!(research.abs_diff_eq(expected, TABLE_TOLERANCE), "L{light_level}: {research} vs {expected}");
            let measured = flat_grey_multiplier(&settings, NIGHT_MAP_MEAN);
            assert!(measured.abs_diff_eq(expected, MEASURED_MEAN_TOLERANCE), "L{light_level}: {measured} vs {expected}");
        }
    }

    #[test]
    fn strength_zero_keeps_the_luminance_of_the_client_multiplier() {
        for (light_level, _) in TABLE {
            let client = flat_grey_multiplier(&ClientNightLighting { light_level, ..client_settings() }, NIGHT_MAP_MEAN);
            let untinted = flat_grey_multiplier(
                &ClientNightLighting { light_level, overlay_strength: MIN_NIGHT_OVERLAY_STRENGTH, ..client_settings() },
                NIGHT_MAP_MEAN,
            );
            assert!((untinted.x - untinted.z).abs() < f32::EPSILON, "{untinted}");
            let luma = client.dot(NIGHT_OVERLAY_LUMA_WEIGHTS);
            assert!((untinted.x - luma).abs() < 1e-6, "{untinted} vs {luma}");
        }
    }

    #[test]
    fn another_colour_rescales_the_map_and_keeps_its_pattern() {
        let warm = Vec3::new(0.15, 0.05, 0.02);
        let ratio = warm / NIGHT_MAP_MEAN;
        assert!(night_overlay(NIGHT_MAP_MEAN, 1.0, ratio).abs_diff_eq(warm, 1e-6));
        let brighter_cloud = NIGHT_MAP_MEAN * 1.5;
        assert!(night_overlay(brighter_cloud, 1.0, ratio).abs_diff_eq(warm * 1.5, 1e-6));
    }

    #[test]
    fn lights_lerp_over_the_night_map() {
        let night = NIGHT_MAP_MEAN;
        let colour = Vec3::from_array(CLIENT_GENERIC_LIGHT_COLOR.map(|channel| f32::from(channel) / BYTE_MAX));
        let alpha = CLIENT_LIGHT_SPRITE_PEAK_ALPHA;
        // One light: its blend over a transparent clear leaves (colour * alpha, alpha).
        let lights = (colour * alpha).extend(alpha);
        let expected = night * (1.0 - alpha) + colour * alpha;
        assert!(light_buffer_over_night(lights, night).abs_diff_eq(expected, 1e-6));
    }

    #[test]
    fn client_shadows_darken_by_a_quarter_and_do_not_stack() {
        const HALF_COVERAGE: f32 = 0.5;
        const SHADOW_BUFFER_FLOOR: f32 = 0.0;
        let one_shadow = CLIENT_SHADOW_BUFFER_CLEAR * (1.0 - 1.0);
        let two_shadows = one_shadow * (1.0 - 1.0);
        assert_eq!(client_shadow_factor(CLIENT_SHADOW_BUFFER_CLEAR, CLIENT_SHADOW_INTENSITY), 1.0);
        assert_eq!(client_shadow_factor(one_shadow, CLIENT_SHADOW_INTENSITY), 0.75);
        assert_eq!(client_shadow_factor(two_shadows, CLIENT_SHADOW_INTENSITY), 0.75);
        assert_eq!(client_shadow_factor(SHADOW_BUFFER_FLOOR, CLIENT_SHADOW_INTENSITY), 0.75);
        // A texel of half coverage leaves S at 0.5, above the 0.25 the composite reaches: no shadow, as in the client.
        assert_eq!(client_shadow_factor(CLIENT_SHADOW_BUFFER_CLEAR * (1.0 - HALF_COVERAGE), CLIENT_SHADOW_INTENSITY), 1.0);
        // Off: the factor is 1 whatever the buffer holds.
        assert_eq!(client_shadow_factor(SHADOW_BUFFER_FLOOR, NO_CLIENT_SHADOW), 1.0);
    }

    #[test]
    fn night_map_placement_follows_the_focus_along_the_screen_axes() {
        let half_view = Vec2::new(13.0, 9.75);
        let origin = night_map_placement(DVec2::ZERO, half_view);
        assert_eq!(origin, Vec4::new(0.5, 0.375, 0.0, 0.0));
        // One step east and one north (x + 1, z - 1) is sqrt(2) along screen right and none down.
        let east_north = night_map_placement(DVec2::new(1.0, -1.0), half_view);
        assert!((east_north.z - std::f32::consts::SQRT_2 / 26.0).abs() < 1e-6, "{east_north}");
        assert!(east_north.w.abs() < 1e-6, "{east_north}");
        // A whole repeat later the map is where it was.
        let repeat = NIGHT_MAP_WORLD_UNITS_PER_REPEAT * std::f64::consts::SQRT_2;
        let far = night_map_placement(DVec2::new(3000.0 + repeat, 2990.0), half_view);
        let near = night_map_placement(DVec2::new(3000.0, 2990.0), half_view);
        assert!((far.z - near.z).abs() < 1e-5 && (far.w - near.w).abs() < 1e-5, "{far} vs {near}");
        // The screen centre samples the focus; the right edge half a view further.
        assert_eq!(night_map_uv(Vec2::splat(0.5), origin), Vec2::ZERO);
        assert_eq!(night_map_uv(Vec2::new(1.0, 0.5), origin), Vec2::new(0.5, 0.0));
    }

    #[test]
    fn measured_mean_constant_matches_the_packed_sums() {
        const TEXELS: f64 = 65536.0;
        const SUMS: [f64; 3] = [210_631.0, 288_859.0, 2_570_545.0];
        let mean = SUMS.map(|sum| (sum / TEXELS / f64::from(BYTE_MAX)) as f32);
        assert!(Vec3::from_array(mean).abs_diff_eq(NIGHT_MAP_MEAN, 1e-6), "{mean:?}");
        assert!(rgba_mean(&[3, 4, 39, 255, 3, 5, 39, 255]).abs_diff_eq(Vec3::new(3.0, 4.5, 39.0) / BYTE_MAX, 1e-6));
    }
}
