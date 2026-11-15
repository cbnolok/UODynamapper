//! Opt-in local test control. No socket or polling system exists unless the
//! caller supplies DYNAMAPPER_CONTROL_SOCKET. The protocol never executes code.
use super::teleport::teleport_player;
use super::InjectedPointer;
mod input;
mod settings;
use input::{ControlInput, InputState, PointerButton, inject_control_input, focus_control_context};
use settings::{ControlSetting, ControlSettings};
use crate::configs::settings::{ClientTextureSource, LandTextureSource};
use crate::core::render::scene::world::land::{LCMesh, draw_mesh::PendingTextureBake};
use crate::core::{render::scene::world::land::{tile_atlas::TileAtlas, LandUploadTelemetry}, texture_cache::land::coverage::EcTerrainBases};
use crate::core::{app_states::AppState, render::scene::{player::Player, world::WorldGeoData, RecomputeVisibleChunksEvent}, system_sets::MovementSysSet};
use crate::prelude::*;
use bevy::{input::{InputSystems, keyboard::KeyCode}, prelude::*, render::view::screenshot::{Screenshot, save_to_disk}, window::PrimaryWindow};
use bevy_egui::EguiPreUpdateSet;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, io, os::unix::{fs::{MetadataExt, PermissionsExt}, net::UnixDatagram}, path::{Path, PathBuf}};

const MAX_COMMAND_BYTES: usize = 4096;
const COMMANDS_PER_FRAME: usize = 8;
/// A pending capture is dropped after this long without its file.
const CAPTURE_PENDING_SECONDS: f64 = 30.0;

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum ControlCommand {
    Teleport { x: u16, y: u16, z: i8, map: u8 },
    Status {},
    Capture { path: PathBuf },
    Key { key: KeyCode, state: InputState, text: Option<String> },
    Text { text: String },
    Pointer { x: f32, y: f32 },
    Click { x: f32, y: f32, button: PointerButton, state: InputState },
    Scroll { x: f32, y: f32 },
    ResetInput {},
    /// The shader window's Render Style (preset group) and Time of Day.
    Preset { style: String, time: String },
    /// The preferences' Art and Land Texture Source.
    Sources { art: ClientTextureSource, land: LandTextureSource },
    /// One whitelisted setting (`settings::ControlSetting`), its value typed by the key.
    Set { key: String, value: Value },
}

/// The last accepted runtime switch (preset, sources or setting): readiness after a switch counts frames from it.
#[derive(Default)]
struct ControlSwitch {
    serial: u64,
    frame: u64,
}

#[derive(Resource)]
struct ControlSocket {
    socket: UnixDatagram,
    path: PathBuf,
    identity: (u64, u64),
}

impl ControlSocket {
    fn bind(path: PathBuf) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "control socket path must be absolute"));
        }
        // bind refuses existing files/sockets; never unlink a caller's file.
        let socket = UnixDatagram::bind(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let bound = Self { socket, path, identity: (metadata.dev(), metadata.ino()) };
        fs::set_permissions(&bound.path, fs::Permissions::from_mode(0o600))?;
        bound.socket.set_nonblocking(true)?;
        Ok(bound)
    }
}

impl Drop for ControlSocket {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path) {
            if (metadata.dev(), metadata.ino()) == self.identity {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

pub struct DevControlPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DevControlPlugin);

impl Plugin for DevControlPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        let Some(path) = std::env::var_os("DYNAMAPPER_CONTROL_SOCKET") else { return; };
        let socket = ControlSocket::bind(PathBuf::from(path))
            .unwrap_or_else(|error| panic!("Cannot bind development control socket: {error}"));
        log::info!("Development control socket: {}", socket.path.display());
        // Readiness after a switch includes the render pipelines it makes Bevy compile.
        crate::core::export_job::count_waiting_render_pipelines(app);
        app.insert_resource(socket)
            .init_resource::<ControlInput>()
            .init_resource::<InjectedPointer>()
            .add_systems(PreUpdate, inject_control_input.before(InputSystems).run_if(in_state(AppState::InGame)))
            .add_systems(PreUpdate, focus_control_context.after(EguiPreUpdateSet::ProcessInput)
                .before(EguiPreUpdateSet::BeginPass).run_if(in_state(AppState::InGame)))
            .add_systems(Update,
            poll_control.in_set(MovementSysSet::MovementActions).run_if(in_state(AppState::InGame)));
    }
}

fn capture_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.extension().and_then(|v| v.to_str()) != Some("png") {
        return Err("capture requires an absolute .png path".into());
    }
    if path.exists() || path.symlink_metadata().is_ok() {
        return Err("capture destination already exists".into());
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err("capture parent directory does not exist".into());
    }
    Ok(())
}

fn poll_control(
    socket: Res<ControlSocket>,
    mut commands: Commands,
    mut player_query: Query<(&mut Player, &mut Transform)>,
    world: Res<WorldGeoData>,
    mut recompute: MessageWriter<RecomputeVisibleChunksEvent>,
    frame_rate: Res<crate::core::frame_rate::MeasuredFrameRate>,
    atlas: Option<Res<TileAtlas>>,
    uploads: Option<Res<LandUploadTelemetry>>,
    terrain_bases: Option<Res<EcTerrainBases>>,
    view: (Query<(Entity, &Window), With<PrimaryWindow>>,
        Query<&Text, With<crate::core::render::overlays::cursor_behavior::OverlayCursorInspectText>>,
        Query<(Has<Mesh3d>, Has<PendingTextureBake>), With<LCMesh>>),
    mut input: ResMut<ControlInput>,
    pointer: Res<InjectedPointer>,
    locals: (Local<Option<(PathBuf, f64)>>, Local<ControlSwitch>),
    time: Res<Time<Real>>,
    mut control_settings: ControlSettings,
    render_state: (
        Option<Res<crate::core::render::scene::world::art::static_lights::RenderStaticLightInstances>>,
        Option<Res<crate::core::render::scene::world::art::statics_collect::StaticArtCollectDebugState>>,
        Option<Res<crate::core::texture_cache::art::SpriteArtPageAtlas>>,
        Option<Res<crate::core::texture_cache::art::GroundArtPageAtlas>>,
        Option<Res<crate::core::render::scene::world::art::gpu_picking::VisibleStaticPick>>,
        Option<Res<crate::core::render::scene::world::land::picking::TerrainCursor>>,
        Option<Res<crate::core::render::scene::world::art::client_light_sprites::RenderClientLightSprites>>,
        Option<Res<crate::core::render::scene::world::z_view::ZViewLimit>>,
        Option<Res<crate::core::render::scene::world::caves::CavesRes>>),
) {
    let (static_lights, art_stats, sprite_atlas, ground_atlas, picking, terrain_cursor, light_sprites, z_view,
        caves) = render_state;
    let (windows, inspect_text, land_chunks) = view;
    let (mut capture_pending, mut switch) = locals;
    if capture_pending.as_ref().is_some_and(|(path, started)|
        path.exists() || time.elapsed_secs_f64() - started > CAPTURE_PENDING_SECONDS) {
        *capture_pending = None;
    }
    let mut buffer = [0u8; MAX_COMMAND_BYTES + 1];
    for _ in 0..COMMANDS_PER_FRAME {
        let (size, peer) = match socket.socket.recv_from(&mut buffer) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => { log::warn!("Development control receive failed: {error}"); break; }
        };
        let Some(peer) = peer.as_pathname() else { continue; };
        let result: Result<Value, String> = (|| {
            if size > MAX_COMMAND_BYTES { return Err("command exceeds 4096 bytes".into()); }
            let command: ControlCommand = serde_json::from_slice(&buffer[..size]).map_err(|error| error.to_string())?;
            match command {
                ControlCommand::Teleport { x, y, z, map } => {
                    let (mut player, mut transform) = player_query.single_mut().map_err(|error| error.to_string())?;
                    teleport_player(&mut player, &mut transform, UOVec4::new(x, y, z, map), &world)?;
                    // Same-map teleports also invalidate the previous visible chunks.
                    recompute.write(RecomputeVisibleChunksEvent {});
                    Ok(json!({"ok": true, "position": [x, y, z, map]}))
                }
                ControlCommand::Status {} => {
                    let (player, _) = player_query.single().map_err(|error| error.to_string())?;
                    let position = player.current_pos.map(|p| json!([p.x, p.y, p.z, p.m]));
                    let rendered = player.prev_rendered_pos.map(|p| json!([p.x, p.y, p.z, p.m]));
                    // A land source switch re-bakes every chunk's texture slots; chunks whose textures are still
                    // streaming stay pending too.
                    let (mut unbuilt_chunks, mut pending_bakes) = (0, 0);
                    for (has_mesh, pending) in &land_chunks {
                        unbuilt_chunks += usize::from(!has_mesh);
                        pending_bakes += usize::from(pending);
                    }
                    let terrain = atlas.as_ref().zip(uploads.as_ref()).map(|(atlas, uploads)| {
                        let uploads = uploads.snapshot();
                        json!({"metadata_revision": atlas.content_revision,
                            "submitted_metadata_revision": uploads.submitted_metadata_revision,
                            "metadata_pending": atlas.has_pending_uploads() || uploads.pending_ops != 0,
                            "cached_chunks": terrain_bases.as_ref().map(|bases| bases.cached_chunk_count()),
                            "unbuilt_chunks": unbuilt_chunks, "pending_bakes": pending_bakes})
                    });
                    let uniforms = control_settings.uniforms();
                    let settings = control_settings.settings();
                    // Terrain metadata can settle before sprite pages and
                    // collection retries. Capture clients must wait for both.
                    let art = art_stats.as_ref().and_then(|state| state.last.as_ref()).map(|stats| json!({
                        "map_id": stats.map_id, "visible_chunks": stats.visible_chunks,
                        "atlas_misses": stats.atlas_misses, "emitted_instances": stats.emitted_instances,
                        "pending_pages": sprite_atlas.as_ref().map_or(0, |a| a.pending_page_count())
                            + ground_atlas.as_ref().map_or(0, |a| a.pending_page_count()),
                    }));
                    Ok(json!({"ok": true, "position": position, "rendered_position": rendered,
                        "elapsed_seconds": time.elapsed_secs_f64(),
                        "fps": frame_rate.frames_per_second(),
                        "frame_ms": frame_rate.frame_ms(), "total_frames": frame_rate.total_frames(),
                        "terrain": terrain, "art": art,
                        "picking": picking.as_ref().map(|p| {
                            let (serial, pending, ready, pixel, render_seen, encoded) = p.status();
                            json!({"serial": serial, "pending": pending, "ready": ready, "pixel": pixel,
                                "render_seen_serial": render_seen, "encoded_serial": encoded})
                        }),
                        "terrain_cursor": terrain_cursor.as_ref().map(|cursor| json!({
                            "pointer": cursor.pointer, "hit": cursor.hit.map(|hit| json!({
                                "logical": [hit.logical.x as i32, hit.logical.y as i32, hit.logical.z as i32, hit.logical.m as i32],
                                "graphic": hit.graphic, "world_position": hit.world_position}))})),
                        "render": uniforms.as_ref().map(|state| json!({
                            "preset": state.active_preset, "global_lighting": state.global_lighting,
                            "effects": state.effects, "local_lights": state.local_lights,
                            "land_texture_variation": state.land_texture_variation,
                            "light_cover": state.light_cover, "light_cover_rule": format!("{:?}", state.light_cover_rule()),
                            "player_floor_rule": state.player_floor_rule,
                            "local_light_intensity": state.effects.local_light_intensity,
                            "fog_params": state.lighting.fog_params, "uniforms_dirty": state.dirty})),
                        "static_lights": static_lights.as_ref().map(|lights| json!({"instances": lights.0.len()})),
                        "client_light_sprites": light_sprites.as_ref().map(|sprites| json!({"instances": sprites.instances.len()})),
                        "z_view": z_view.as_ref().map(|limit| json!({"max_z": limit.0.max_z,
                            "max_ground_z": limit.0.max_ground_z, "hide_roofs": limit.0.hide_roofs,
                            "roof_layer": limit.0.roof_layer})),
                        "cave_surroundings": settings.map(|settings| settings.world_rendering.cave_surroundings),
                        "control": control_settings.status().as_object().cloned().map(|mut control| {
                            control.insert("switch_serial".into(), switch.serial.into());
                            control.insert("frames_since_switch".into(),
                                frame_rate.total_frames().saturating_sub(switch.frame).into());
                            control
                        }),
                        "player_cave": caves.as_ref().and_then(|caves| caves.player()).map(|cave| json!({
                            "floor_z": cave.floor_z(), "cells": cave.inside_cells()})),
                        "input": {"queued_frames": input.queued_frames(), "context_focus_override": input.focused,
                            "pointer": pointer.0.map(|position| [position.x, position.y])},
                        "cursor_inspect": inspect_text.single().ok().map(|text| &text.0),
                        "window": windows.single().ok().map(|(_, window)| json!({
                            "logical_size": [window.width(), window.height()], "scale_factor": window.scale_factor(),
                            "focused": window.focused}))}))
                }
                ControlCommand::Capture { path } => {
                    capture_path(&path)?;
                    if input.queued_frames() != 0 { return Err("input is still queued; wait for input.queued_frames == 0".into()); }
                    if capture_pending.is_some() { return Err("another capture is pending".into()); }
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
                    *capture_pending = Some((path.clone(), time.elapsed_secs_f64()));
                    Ok(json!({"ok": true, "capture_queued": path}))
                }
                ControlCommand::Preset { style, time } => {
                    let changed = control_settings.set_preset(&style, &time)?;
                    Ok(switched(&mut switch, changed, frame_rate.total_frames(),
                        json!({"preset": format!("{style}.{time}")})))
                }
                ControlCommand::Sources { art, land } => {
                    let changed = control_settings.set_sources(art, land)?;
                    Ok(switched(&mut switch, changed, frame_rate.total_frames(), json!({"art": art, "land": land})))
                }
                ControlCommand::Set { key, value } => {
                    let setting = ControlSetting::parse(&key, value)?;
                    let changed = control_settings.set(setting, &mut recompute)?;
                    Ok(switched(&mut switch, changed, frame_rate.total_frames(), json!({"key": key})))
                }
                command => {
                    let (entity, window) = windows.single().map_err(|error| error.to_string())?;
                    input.enqueue(command, entity, Vec2::new(window.width(), window.height()))?;
                    Ok(json!({"ok": true, "input_queued_frames": input.queued_frames()}))
                }
            }
        })();
        let reply = result.unwrap_or_else(|error| json!({"ok": false, "error": error}));
        if let Err(error) = socket.socket.send_to(reply.to_string().as_bytes(), peer) {
            log::debug!("Development control reply failed: {error}");
        }
    }
}

/// The reply to an accepted switch; a change restarts the frame count readiness waits on.
fn switched(switch: &mut ControlSwitch, changed: bool, frame: u64, applied: Value) -> Value {
    if changed {
        switch.serial += 1;
        switch.frame = frame;
    }
    json!({"ok": true, "changed": changed, "applied": applied, "switch_serial": switch.serial})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn protocol_rejects_unknown_actions_fields_and_coordinate_overflow() {
        for value in [r#"{"command":"execute","code":"bad"}"#, r#"{"command":"status","extra":true}"#,
            r#"{"command":"key","key":"F2","state":"repeat"}"#,
            r#"{"command":"key","key":"BadKey","state":"tap"}"#,
            r#"{"command":"click","x":1,"y":1,"button":"bad","state":"tap"}"#,
            r#"{"command":"teleport","x":65536,"y":561,"z":0,"map":0}"#] {
            assert!(serde_json::from_str::<ControlCommand>(value).is_err(), "{value}");
        }
        assert!(matches!(serde_json::from_str::<ControlCommand>(r#"{"command":"teleport","x":2518,"y":561,"z":0,"map":0}"#).unwrap(), ControlCommand::Teleport { x: 2518, y: 561, .. }));
    }

    #[test]
    fn switch_commands_are_typed_and_refuse_missing_extra_and_unknown_values() {
        for value in [r#"{"command":"preset","style":"kr"}"#,
            r#"{"command":"preset","style":"kr","time":"night","save":true}"#,
            r#"{"command":"sources","art":"kr","land":"ec"}"#, r#"{"command":"sources","art":"cc"}"#,
            r#"{"command":"sources","art":"cc","land":"uop"}"#, r#"{"command":"set","key":"caves"}"#,
            r#"{"command":"set","key":"caves","value":"black","persist":true}"#] {
            assert!(serde_json::from_str::<ControlCommand>(value).is_err(), "{value}");
        }
        assert!(matches!(serde_json::from_str::<ControlCommand>(r#"{"command":"sources","art":"cc","land":"kr"}"#)
            .unwrap(), ControlCommand::Sources { art: ClientTextureSource::Cc, land: LandTextureSource::Kr }));
        let ControlCommand::Set { key, value } = serde_json::from_str::<ControlCommand>(
            r#"{"command":"set","key":"show_only_player_floor","value":true}"#).unwrap() else { panic!("set") };
        assert_eq!(ControlSetting::parse(&key, value).unwrap(), ControlSetting::ShowOnlyPlayerFloor(true));
        let ControlCommand::Preset { style, time } = serde_json::from_str::<ControlCommand>(
            r#"{"command":"preset","style":"painterly","time":"cave"}"#).unwrap() else { panic!("preset") };
        assert_eq!((style.as_str(), time.as_str()), ("painterly", "cave"));
    }

    #[test]
    fn only_a_change_restarts_the_switch_frame_count() {
        let mut switch = ControlSwitch::default();
        let reply = switched(&mut switch, false, 10, json!({}));
        assert_eq!((switch.serial, switch.frame, reply["changed"].as_bool()), (0, 0, Some(false)));
        let reply = switched(&mut switch, true, 12, json!({}));
        assert_eq!((switch.serial, switch.frame, reply["switch_serial"].as_u64()), (1, 12, Some(1)));
    }

    #[test]
    fn socket_is_private_refuses_existing_paths_and_cleans_only_its_own_inode() {
        let root = std::env::temp_dir().join(format!("dynamapper-control-{}-{}", std::process::id(), NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&root).unwrap();
        let path = root.join("control.sock");
        let server = ControlSocket::bind(path.clone()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(ControlSocket::bind(path.clone()).is_err());
        let client_path = root.join("client.sock");
        let client = UnixDatagram::bind(&client_path).unwrap();
        client.send_to(b"status", &path).unwrap();
        let mut buffer = [0u8; 16];
        assert_eq!(server.socket.recv_from(&mut buffer).unwrap().0, 6);
        fs::remove_file(&path).unwrap();
        fs::write(&path, "replacement").unwrap();
        drop(server);
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        assert!(ControlSocket::bind(path.clone()).is_err());
        fs::remove_file(&path).unwrap();
        drop(ControlSocket::bind(path.clone()).unwrap());
        assert!(!path.exists());
        drop(client);
        fs::remove_file(client_path).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn capture_refuses_existing_files_symlinks_and_invalid_destinations() {
        let root = std::env::temp_dir().join(format!("dynamapper-capture-{}-{}", std::process::id(), NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&root).unwrap();
        let path = root.join("reference.png");
        fs::write(&path, "reference").unwrap();
        assert!(capture_path(&path).is_err());
        let link = root.join("alias.png");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(capture_path(&link).is_err());
        assert!(capture_path(Path::new("relative.png")).is_err());
        assert!(capture_path(&root.join("missing/capture.png")).is_err());
        assert!(capture_path(&root.join("capture.jpg")).is_err());
        assert!(capture_path(&root.join("new.png")).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "reference");
        fs::remove_file(link).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
