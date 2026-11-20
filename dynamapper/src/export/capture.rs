//! The capture: the world camera pointed at an offscreen target, the frame placed and its pixels cut out of the
//! capture, and the check that the camera put the anchor tile where the projection says.

use crate::render::view::camera::{ORTHO_SIZE_FACTOR, ORTHO_WIDTH_SCALE_FACTOR, PlayerCamera};
use crate::world::teleport::teleport_player;
use bevy::prelude::*;
use bevy::{
    camera::{ImageRenderTarget, RenderTarget, ScalingMode},
    image::Image,
    render::{
        render_resource::{TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use color_eyre::eyre;
use super::job::{EXPORT_ZOOM, ExportJob, ExportJobFrame};
use super::placement::{CAPTURE_PAD_PX, FramePlacement, project};
use super::{ExportRuntime, ExportScene};

const RGBA_BYTES_PER_PIXEL: usize = 4;
/// How far the measured check-tile vertex may sit from a whole pixel before the placement is an error.
const PLACEMENT_TOLERANCE_PX: f32 = 0.05;

/// The pixels of the frame being captured, delivered by the screenshot observer.
#[derive(Resource, Default)]
pub(super) struct ExportCapture(pub(super) Option<Image>);

pub(super) fn request_capture(commands: &mut Commands, runtime: &ExportRuntime) {
    let target = runtime.target.clone().expect("the export target");
    commands.spawn(Screenshot::image(target)).observe(
        |captured: On<ScreenshotCaptured>, mut capture: ResMut<ExportCapture>| {
            capture.0 = Some(captured.image.clone());
        },
    );
}

/// Points the world camera at an offscreen target, the frame plus its pad, with an opaque black background.
pub(super) fn setup_export_camera(job: &ExportJob, scene: &mut ExportScene) -> eyre::Result<Handle<Image>> {
    let target_size = capture_size_px(job.frame_size_px);
    let mut image = Image::new_target_texture(target_size, target_size, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = scene.images.add(image);
    let (mut camera, mut projection, _, mut target, _) =
        scene.camera.single_mut().map_err(|error| eyre::eyre!("world camera: {error}"))?;
    camera.clear_color = ClearColorConfig::Custom(Color::BLACK);
    *target = RenderTarget::Image(ImageRenderTarget { handle: handle.clone(), scale_factor: 1.0 });
    let Projection::Orthographic(ortho) = &mut *projection else {
        eyre::bail!("the export needs the orthographic camera");
    };
    // The same pixel scale as the window: `sys_update_camera_projection_to_view` at the target's size.
    let size = target_size as f32;
    ortho.scale = EXPORT_ZOOM;
    ortho.scaling_mode = ScalingMode::Fixed {
        width: size / ORTHO_SIZE_FACTOR,
        height: (size / ORTHO_WIDTH_SCALE_FACTOR) / ORTHO_SIZE_FACTOR,
    };
    Ok(handle)
}

pub(super) fn place_frame(job: &ExportJob, placement: &FramePlacement, scene: &mut ExportScene) -> eyre::Result<()> {
    let (_, _, mut transform, _, _) =
        scene.camera.single_mut().map_err(|error| eyre::eyre!("world camera: {error}"))?;
    let look_at = Vec3::new(placement.look_at_tile.0 as f32, 0.0, placement.look_at_tile.1 as f32);
    *transform = Transform::from_translation(look_at + PlayerCamera::BASE_OFFSET_FROM_PLAYER).looking_at(look_at, Vec3::Y);
    let (mut player, mut player_transform) =
        scene.player.single_mut().map_err(|error| eyre::eyre!("player: {error}"))?;
    teleport_player(&mut player, &mut player_transform, placement.player_position(job), &scene.world)
        .map_err(|error| eyre::eyre!(error))
}

/// Where the camera actually put the anchor tile's top vertex, in base-level pixels.
pub(super) fn measured_vertex(
    job: &ExportJob,
    frame: &ExportJobFrame,
    placement: &FramePlacement,
    scene: &ExportScene,
) -> eyre::Result<[i64; 2]> {
    let (camera, _, _, _, camera_transform) = scene.camera.single().map_err(|error| eyre::eyre!("world camera: {error}"))?;
    let (x, y) = placement.anchor_tile;
    let viewport = camera
        .world_to_viewport(camera_transform, Vec3::new(x as f32, 0.0, y as f32))
        .map_err(|error| eyre::eyre!("project the check tile: {error:?}"))?;
    let rounded = viewport.round();
    if (viewport - rounded).abs().max_element() > PLACEMENT_TOLERANCE_PX {
        eyre::bail!("frame {}: tile {x},{y} landed at {viewport}, off the pixel grid", frame.id);
    }
    let measured = [
        frame.left_px + rounded.x as i64 - i64::from(placement.crop_origin.0),
        frame.top_px + rounded.y as i64 - i64::from(placement.crop_origin.1),
    ];
    let expected = project(job, i64::from(x), i64::from(y));
    if (measured[0], measured[1]) != expected {
        eyre::bail!("frame {}: tile {x},{y} landed at {measured:?}, the projection says {expected:?}", frame.id);
    }
    Ok(measured)
}

pub(super) fn capture_size_px(frame_size_px: u32) -> u32 {
    frame_size_px + 2 * CAPTURE_PAD_PX
}

/// The frame cut out of the capture, as RGBA8 rows top to bottom.
pub(super) fn frame_pixels(image: Image, frame_size_px: u32, crop_origin: (u32, u32)) -> eyre::Result<Vec<u8>> {
    let format = image.texture_descriptor.format;
    let size = image.texture_descriptor.size;
    let data = image.data.ok_or_else(|| eyre::eyre!("the capture has no pixel data"))?;
    let target_size = capture_size_px(frame_size_px);
    let expected = target_size as usize * target_size as usize * RGBA_BYTES_PER_PIXEL;
    if (size.width, size.height) != (target_size, target_size) || data.len() != expected {
        eyre::bail!("the capture is {}x{} with {} bytes, not a {target_size}² RGBA8 image", size.width, size.height, data.len());
    }
    let (row_bytes, frame_row_bytes) =
        (target_size as usize * RGBA_BYTES_PER_PIXEL, frame_size_px as usize * RGBA_BYTES_PER_PIXEL);
    let mut data: Vec<u8> = (0..frame_size_px as usize)
        .flat_map(|row| {
            let start = (crop_origin.1 as usize + row) * row_bytes + crop_origin.0 as usize * RGBA_BYTES_PER_PIXEL;
            data[start..start + frame_row_bytes].iter().copied()
        })
        .collect();
    match format {
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => {}
        TextureFormat::Bgra8UnormSrgb | TextureFormat::Bgra8Unorm => {
            data.chunks_exact_mut(RGBA_BYTES_PER_PIXEL).for_each(|pixel| pixel.swap(0, 2));
        }
        other => eyre::bail!("the capture format {other:?} is not 8-bit RGBA"),
    }
    Ok(data)
}
