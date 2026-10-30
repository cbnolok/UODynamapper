//! Opt-in local test control. No socket or polling system exists unless the
//! caller supplies DYNAMAPPER_CONTROL_SOCKET. The protocol never executes code.
use super::teleport::teleport_player;
use super::InjectedPointer;
mod input;
use input::{ControlInput, InputState, PointerButton, inject_control_input, focus_control_context};
use crate::core::{render::scene::world::land::{tile_atlas::TileAtlas, LandUploadTelemetry}, texture_cache::land::coverage::EcTerrainBases};
use crate::core::{app_states::AppState, render::scene::{player::Player, world::WorldGeoData, RecomputeVisibleChunksEvent}, system_sets::MovementSysSet};
use crate::prelude::*;
use bevy::{diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin}, input::{InputSystems, keyboard::KeyCode}, prelude::*, render::view::screenshot::{Screenshot, save_to_disk}, window::PrimaryWindow};
use bevy_egui::EguiPreUpdateSet;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, io, os::unix::{fs::{MetadataExt, PermissionsExt}, net::UnixDatagram}, path::{Path, PathBuf}};

const MAX_COMMAND_BYTES: usize = 4096;
const COMMANDS_PER_FRAME: usize = 8;

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
    diagnostics: Option<Res<DiagnosticsStore>>,
    atlas: Option<Res<TileAtlas>>,
    uploads: Option<Res<LandUploadTelemetry>>,
    terrain_bases: Option<Res<EcTerrainBases>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut input: ResMut<ControlInput>,
    pointer: Res<InjectedPointer>,
    mut capture_pending: Local<Option<(PathBuf, f64)>>,
    time: Res<Time<Real>>,
    uniforms: Option<Res<crate::configs::shader_presets::UniformState>>,
    static_lights: Option<Res<crate::core::render::scene::world::art::static_lights::RenderStaticLightInstances>>,
) {
    if capture_pending.as_ref().is_some_and(|(path, started)| path.exists() || time.elapsed_secs_f64() - started > 30.0) {
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
                    let fps_path = FrameTimeDiagnosticsPlugin::FPS;
                    let frame_path = FrameTimeDiagnosticsPlugin::FRAME_TIME;
                    let metric = |path| diagnostics.as_ref().and_then(|store| store.get(path)).and_then(|value| value.smoothed());
                    let terrain = atlas.as_ref().zip(uploads.as_ref()).map(|(atlas, uploads)| {
                        let uploads = uploads.snapshot();
                        json!({"metadata_revision": atlas.content_revision,
                            "submitted_metadata_revision": uploads.submitted_metadata_revision,
                            "metadata_pending": atlas.has_pending_uploads() || uploads.pending_ops != 0,
                            "cached_chunks": terrain_bases.as_ref().map(|bases| bases.cached_chunk_count())})
                    });
                    Ok(json!({"ok": true, "position": position, "rendered_position": rendered,
                        "elapsed_seconds": time.elapsed_secs_f64(),
                        "fps": metric(&fps_path),
                        "frame_ms": metric(&frame_path), "terrain": terrain,
                        "render": uniforms.as_ref().map(|state| json!({
                            "preset": state.active_preset, "global_lighting": state.global_lighting,
                            "effects": state.effects})),
                        "static_lights": static_lights.as_ref().map(|lights| json!({"instances": lights.0.len()})),
                        "input": {"queued_frames": input.queued_frames(), "context_focus_override": input.focused,
                            "pointer": pointer.0.map(|position| [position.x, position.y])},
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
