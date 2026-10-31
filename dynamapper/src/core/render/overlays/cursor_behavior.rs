use crate::core::controls::input_actions::{
    ActionToggleCursorInspectPanel, ActionToggleCursorTeleportMode,
};
use crate::core::controls::{InjectedPointer, effective_cursor_position};
use crate::core::render::scene::world::art::statics_collect::{
    resolve_priority_z_units, resolve_static_depth_class, static_depth_key,
    static_tile_is_surface_like, static_tile_renders_wet,
};
use crate::core::render::scene::world::art::static_geometry::{StaticArtRoute, StaticPickingResources, projected_quad_hit};
use crate::core::render::scene::player::Player;
use crate::core::render::{
    dialogs,
    scene::{
        camera::{PlayerCamera, UiCameraResource},
        world::WorldGeoData,
        RecomputeVisibleChunksEvent,
    },
};
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{
    TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, MapPlanesRes, TileMetaPackageRes,
};
use crate::ingame_sysmessage_logger;
use crate::prelude::*;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::text::{FontSmoothing, LineHeight};
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;
use uocf::classic::map::{MapCell, MapCellCoords};

const FONT_SIZE: f32 = 13.0;
const CLASSIC_STATIC_ART_ID_OFFSET: u16 = 0x4000;
const HOVERED_STATIC_HIGHLIGHT_Y_LIFT: f32 = 0.01;

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

#[derive(Resource, Default)]
pub struct CursorInspectOverlayState {
    pub enabled: bool,
}

pub struct CursorBehaviorOverlayPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(CursorBehaviorOverlayPlugin);

#[derive(Component)]
pub struct OverlayCursorBehaviorContainer;

#[derive(Component)]
pub struct OverlayCursorModeText;

#[derive(Component)]
pub struct OverlayCursorPositionText;

#[derive(Component)]
pub struct OverlayCursorInspectContainer;

#[derive(Component)]
pub struct OverlayCursorInspectText;

#[derive(SystemParam)]
pub struct CursorInspectResources<'w> {
    settings: Res<'w, crate::configs::settings::Settings>,
    inspect_state: Res<'w, CursorInspectOverlayState>,
    map_planes_r: Res<'w, MapPlanesRes>,
    statics_res: Res<'w, StaticsStoreRes>,
    tex_art_cc_res: Option<Res<'w, TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<'w, TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<'w, TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<'w, TileMetaPackageRes>>,
}

#[derive(SystemParam)]
pub struct CursorInspectLocals<'w, 's> {
    last_show_inspect: Local<'s, bool>,
    last_scale: Local<'s, f32>,
    last_cursor_pos: Local<'s, Option<Vec2>>,
    last_window_size: Local<'s, Option<(f32, f32)>>,
    last_camera_view: Local<'s, Option<(Mat4, Mat4)>>,
    last_player_map_id: Local<'s, Option<u8>>,
    _marker: std::marker::PhantomData<&'w ()>,
}

impl Plugin for CursorBehaviorOverlayPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<CursorBehavior>()
            .init_resource::<CursorInspectOverlayState>()
            .add_observer(sys_toggle_cursor_mode)
            .add_observer(sys_toggle_cursor_inspect_panel)
            .add_systems(OnEnter(AppState::InGame), setup_overlay_cursor_behavior)
            .add_systems(
                Update,
                (
                    update_cursor_behavior_text.run_if(in_state(AppState::InGame)),
                    update_cursor_inspect_text.after(crate::core::system_sets::SceneRenderArtSysSet::CollectVisibleStatics)
                        .run_if(in_state(AppState::InGame)),
                    sys_draw_hovered_static_highlight.after(crate::core::system_sets::SceneRenderArtSysSet::CollectVisibleStatics)
                        .run_if(in_state(AppState::InGame)),
                    sys_teleport_on_click.run_if(in_state(AppState::InGame)),
                ),
            );
    }
}

pub fn setup_overlay_cursor_behavior(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<crate::configs::settings::Settings>,
) {
    crate::util_lib::tracked_plugin::log_system_add_one_shot::<CursorBehaviorOverlayPlugin>(
        "OnEnter(InGame)",
        "None",
        crate::fname!(),
    );
    let font: Handle<Font> = asset_server.load("fonts/uo/UOClassicRough.ttf");

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(108.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Start,
                justify_content: JustifyContent::Start,
                padding: UiRect::all(Val::Px(7.0 * settings.app.window.cursor_position_scale)),
                display: if settings.app.performance.cursor_inspect_overlay_visible() {
                    Display::Flex
                } else {
                    Display::None
                },
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            GlobalZIndex(100),
            OverlayCursorBehaviorContainer,
        ))
        .with_children(|builder| {
            let scale = settings.app.window.cursor_position_scale;
            builder.spawn((
                Text::new("Cursor mode: Select"),
                TextFont {
                    font: font.clone().into(),
                    font_size: (FONT_SIZE * scale).into(),
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..default()
                },
                LineHeight::Px(FONT_SIZE * scale),
                TextColor(Color::WHITE),
                OverlayCursorModeText,
            ));
            builder.spawn((
                Text::new("Cursor position: (NA, NA, NA)"),
                TextFont {
                    font: font.clone().into(),
                    font_size: (FONT_SIZE * scale).into(),
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..default()
                },
                LineHeight::Px(FONT_SIZE * scale),
                TextColor(Color::WHITE),
                OverlayCursorPositionText,
            ));
            builder
                .spawn((
                    Node {
                        margin: UiRect::top(Val::Px(6.0 * scale)),
                        display: Display::None,
                        ..default()
                    },
                    OverlayCursorInspectContainer,
                ))
                .with_children(|builder| {
                    builder.spawn((
                        Text::new("Hovered tiles:\n[disabled]"),
                        TextFont {
                            font: font.clone().into(),
                            font_size: (FONT_SIZE * scale).into(),
                            font_smoothing: FontSmoothing::AntiAliased,
                            ..default()
                        },
                        LineHeight::Px(FONT_SIZE * scale),
                        TextColor(Color::WHITE),
                        OverlayCursorInspectText,
                    ));
                });
        });
}

pub fn update_cursor_behavior_text(
    settings: Res<crate::configs::settings::Settings>,
    cursor: Res<CursorBehavior>,
    player_q: Query<&Player>,
    windows: (Query<&Window, With<PrimaryWindow>>, Option<Res<InjectedPointer>>),
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    map_planes_r: Res<MapPlanesRes>,
    mut mode_text_q: Query<
        (&mut Text, &mut TextFont, &mut LineHeight),
        (
            With<OverlayCursorModeText>,
            Without<OverlayCursorPositionText>,
        ),
    >,
    mut position_text_q: Query<
        (&mut Text, &mut TextFont, &mut LineHeight),
        (
            With<OverlayCursorPositionText>,
            Without<OverlayCursorModeText>,
        ),
    >,
    mut node_q: Query<&mut Node, With<OverlayCursorBehaviorContainer>>,
    mut last_show_overlay: Local<bool>,
    mut last_scale: Local<f32>,
    mut last_mode: Local<Option<CursorMode>>,
    mut last_cursor_pos: Local<Option<Vec2>>,
    mut last_window_size: Local<Option<(f32, f32)>>,
    mut last_camera_translation: Local<Option<Vec3>>,
    mut last_player_map_id: Local<Option<u8>>,
) {
    let (windows, injected_pointer) = windows;
    let current_scale = settings.app.window.cursor_position_scale;
    let scale_changed = (*last_scale - current_scale).abs() > 0.001;

    let show_overlay = settings.app.performance.cursor_inspect_overlay_visible();
    if *last_show_overlay != show_overlay || scale_changed {
        if let Ok(mut node) = node_q.single_mut() {
            node.display = if show_overlay {
                Display::Flex
            } else {
                Display::None
            };
            node.padding = UiRect::all(Val::Px(7.0 * current_scale));
        }
        *last_show_overlay = show_overlay;
    }

    if scale_changed {
        if let Ok((_, mut text_font, mut line_height)) = mode_text_q.single_mut() {
            text_font.font_size = (FONT_SIZE * current_scale).into();
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
        }
        if let Ok((_, mut text_font, mut line_height)) = position_text_q.single_mut() {
            text_font.font_size = (FONT_SIZE * current_scale).into();
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
        }
        *last_scale = current_scale;
    }

    if *last_mode != Some(cursor.mode) {
        if let Ok((mut mode_text, _, _)) = mode_text_q.single_mut() {
            let next_text = format!(
                "Cursor mode: {}",
                match cursor.mode {
                    CursorMode::Select => "Select",
                    CursorMode::Teleport => "Teleport",
                }
            );
            if mode_text.0 != next_text {
                mode_text.0 = next_text;
            }
        }
        *last_mode = Some(cursor.mode);
    }

    let window_size = windows
        .single()
        .ok()
        .map(|window| (window.width(), window.height()));
    let camera_translation = camera_q
        .single()
        .ok()
        .map(|(_, camera_tf)| camera_tf.translation());
    let cursor_pos = windows
        .single()
        .ok()
        .and_then(|window| effective_cursor_position(window, injected_pointer.as_deref()));
    let player_map_id = player_q
        .single()
        .ok()
        .and_then(|player| player.current_pos.map(|p| p.m));

    let needs_rebuild = *last_cursor_pos != cursor_pos
        || *last_window_size != window_size
        || *last_camera_translation != camera_translation
        || *last_player_map_id != player_map_id;

    if needs_rebuild {
        let cursor_position_label =
            match (cursor_pos, camera_q.single().ok(), player_q.single().ok()) {
                (Some(cursor_pos), Some((camera, camera_tf)), Some(player)) => {
                    match camera.viewport_to_world(camera_tf, cursor_pos).ok() {
                        Some(ray) if ray.direction.y.abs() > 1e-6 => {
                            let t = -ray.origin.y / ray.direction.y;
                            let hit = ray.origin + ray.direction * t;
                            let cursor_x = hit.x.round().max(0.0) as u16;
                            let cursor_y = hit.z.round().max(0.0) as u16;
                            let map_id = player
                                .current_pos
                                .map(|p| p.m)
                                .unwrap_or(settings.session_state.world.last_p.m);
                            let cursor_z =
                                resolve_cursor_map_z(&map_planes_r, map_id, cursor_x, cursor_y)
                                    .unwrap_or(0);
                            format!(
                                "Cursor position:\n[{}, {}, {}]",
                                cursor_x, cursor_y, cursor_z
                            )
                        }
                        _ => "Cursor position:\n[NA, NA, NA]".to_string(),
                    }
                }
                _ => "Cursor position:\n[NA, NA, NA]".to_string(),
            };

        if let Ok((mut position_text, _, _)) = position_text_q.single_mut() {
            if position_text.0 != cursor_position_label {
                position_text.0 = cursor_position_label;
            }
        }

        *last_cursor_pos = cursor_pos;
        *last_window_size = window_size;
        *last_camera_translation = camera_translation;
        *last_player_map_id = player_map_id;
    }
}

pub(crate) fn update_cursor_inspect_text(
    resources: CursorInspectResources,
    mut picking: StaticPickingResources,
    player_q: Query<&Player>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut inspect_text_q: Query<
        (&mut Text, &mut TextFont, &mut LineHeight),
        With<OverlayCursorInspectText>,
    >,
    mut inspect_node_q: Query<&mut Node, With<OverlayCursorInspectContainer>>,
    mut locals: CursorInspectLocals,
) {
    let current_scale = resources.settings.app.window.cursor_position_scale;
    let scale_changed = (*locals.last_scale - current_scale).abs() > 0.001;
    let show_inspect =
        resources.settings.app.performance.cursor_inspect_overlay_visible() && resources.inspect_state.enabled;

    if *locals.last_show_inspect != show_inspect || scale_changed {
        if let Ok(mut node) = inspect_node_q.single_mut() {
            node.display = if show_inspect {
                Display::Flex
            } else {
                Display::None
            };
            node.margin = UiRect::top(Val::Px(6.0 * current_scale));
        }
        *locals.last_show_inspect = show_inspect;
    }

    if scale_changed {
        if let Ok((_, mut text_font, mut line_height)) = inspect_text_q.single_mut() {
            text_font.font_size = (FONT_SIZE * current_scale).into();
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
        }
        *locals.last_scale = current_scale;
    }

    let window_size = windows
        .single()
        .ok()
        .map(|window| (window.width(), window.height()));
    let camera_view = camera_q
        .single()
        .ok()
        .map(|(camera, camera_tf)| (camera_tf.to_matrix(), camera.clip_from_view()));
    let cursor_pos = windows
        .single()
        .ok()
        .and_then(|window| effective_cursor_position(window, injected_pointer.as_deref()));
    let player_map_id = player_q
        .single()
        .ok()
        .and_then(|player| player.current_pos.map(|p| p.m));

    let needs_rebuild = *locals.last_cursor_pos != cursor_pos
        || *locals.last_window_size != window_size
        || *locals.last_camera_view != camera_view
        || *locals.last_player_map_id != player_map_id
        || scale_changed
        || resources.inspect_state.is_changed()
        || resources.settings.is_changed()
        || picking.is_changed();

    if !show_inspect || !needs_rebuild {
        return;
    }

    let inspect_label = build_cursor_inspect_label(
        &mut picking,
        cursor_pos,
        camera_q.single().ok(),
        player_q.single().ok(),
        &resources.settings,
        &resources.map_planes_r,
        &resources.statics_res,
        resources.tex_art_cc_res.as_deref(),
        resources.tex_art_ec_res.as_deref(),
        resources.tex_land_ec_res.as_deref(),
        resources.tilemeta_res.as_deref(),
    );

    if let Ok((mut inspect_text, _, _)) = inspect_text_q.single_mut() {
        if inspect_text.0 != inspect_label {
            inspect_text.0 = inspect_label;
        }
    }

    *locals.last_cursor_pos = cursor_pos;
    *locals.last_window_size = window_size;
    *locals.last_camera_view = camera_view;
    *locals.last_player_map_id = player_map_id;
}

pub fn resolve_cursor_map_z(map_planes_r: &MapPlanesRes, map_id: u8, x: u16, y: u16) -> Option<i8> {
    let plane = map_planes_r
        .0
        .get(map_id as usize)
        .and_then(|opt| opt.as_ref())
        .expect("Uncached Map in resolve_cursor_map_z");
    let cell = MapCellCoords {
        x: x as u32,
        y: y as u32,
    };
    let block_pos = MapCell::coords_of_parent_block(&cell);
    let block = plane.block_no_update(block_pos)?;
    let rel = MapCell::coords_in_block(&cell);
    Some(block.cell(rel.x, rel.y).ok()?.z)
}

fn sys_toggle_cursor_mode(
    _trigger: On<ActionToggleCursorTeleportMode>,
    mut cursor: ResMut<CursorBehavior>,
) {
    cursor.mode = match cursor.mode {
        CursorMode::Select => CursorMode::Teleport,
        CursorMode::Teleport => CursorMode::Select,
    };
    ingame_sysmessage_logger::normal(format!(
        "Cursor mode: {}",
        match cursor.mode {
            CursorMode::Select => "Select",
            CursorMode::Teleport => "Teleport",
        }
    ));
}

fn sys_toggle_cursor_inspect_panel(
    _trigger: On<ActionToggleCursorInspectPanel>,
    mut inspect: ResMut<CursorInspectOverlayState>,
) {
    inspect.enabled = !inspect.enabled;
    ingame_sysmessage_logger::normal(format!(
        "Cursor tile inspect: {}",
        if inspect.enabled { "On" } else { "Off" }
    ));
}

fn build_cursor_inspect_label(
    picking: &mut StaticPickingResources,
    cursor_pos: Option<Vec2>,
    camera: Option<(&Camera, &GlobalTransform)>,
    player: Option<&Player>,
    settings: &crate::configs::settings::Settings,
    map_planes_r: &MapPlanesRes,
    statics_res: &StaticsStoreRes,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
) -> String {
    let Some((map_id, cursor_x, cursor_y)) =
        resolve_cursor_tile_coords(cursor_pos, camera, player, settings)
    else {
        return "Hovered tiles:\n[NA]".to_string();
    };

    let hovered_object_line = describe_hovered_object(
        picking,
        cursor_pos,
        camera,
        settings,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
        map_id,
    );
    let land_line = describe_land_tile(map_planes_r, tilemeta_res, map_id, cursor_x, cursor_y);
    let statics_line = describe_static_tiles(
        picking,
        statics_res,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
        map_id,
        cursor_x,
        cursor_y,
    );

    format!(
        "Hovered tiles:\ncell=[{}, {}] map={} art_source={:?}\n{}\n{}\n{}",
        cursor_x,
        cursor_y,
        map_id,
        settings.graphics.art_texture_source,
        hovered_object_line,
        land_line,
        statics_line
    )
}

#[derive(Clone, Copy, Debug)]
enum HoveredObjectKind {
    Sprite,
    Ground,
}

#[derive(Clone)]
struct HoveredStaticMatch {
    tile: uocf::classic::statics::PackedStaticTile,
    kind: HoveredObjectKind,
    corners: [Vec3; 4],
    depth_key: f32,
    global_x: u16,
    global_y: u16,
    route: StaticArtRoute,
}

fn sys_draw_hovered_static_highlight(
    mut picking: StaticPickingResources,
    settings: Res<crate::configs::settings::Settings>,
    player_q: Query<&Player>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    mut gizmos: Gizmos,
) {
    if !settings
        .world_rendering
        .diagnostics
        .highlight_hovered_static_object
    {
        return;
    }

    let Some(cursor_pos) = windows.single().ok().and_then(|window| effective_cursor_position(window, injected_pointer.as_deref())) else {
        return;
    };
    let Some((camera, camera_tf)) = camera_q.single().ok() else {
        return;
    };
    let Some(player) = player_q.single().ok() else {
        return;
    };
    let Some((map_id, _, _)) = resolve_cursor_tile_coords(
        Some(cursor_pos),
        Some((camera, camera_tf)),
        Some(player),
        &settings,
    ) else {
        return;
    };

    let Some(best_match) = find_hovered_static_object(
        &mut picking,
        cursor_pos,
        camera,
        camera_tf,
        &settings,
        tilemeta_res.as_deref(),
        map_id,
    ) else {
        return;
    };

    let color = match best_match.kind {
        HoveredObjectKind::Sprite => Color::srgb(0.1, 1.0, 0.2),
        HoveredObjectKind::Ground => Color::srgb(1.0, 0.85, 0.2),
    };
    let lift = Vec3::new(0.0, HOVERED_STATIC_HIGHLIGHT_Y_LIFT, 0.0);
    let [corner0, corner1, corner2, corner3] = best_match.corners;
    let corner0 = corner0 + lift;
    let corner1 = corner1 + lift;
    let corner2 = corner2 + lift;
    let corner3 = corner3 + lift;

    gizmos.line(corner0, corner1, color);
    gizmos.line(corner1, corner3, color);
    gizmos.line(corner3, corner2, color);
    gizmos.line(corner2, corner0, color);
    gizmos.line(corner0, corner3, color);
    gizmos.line(corner1, corner2, color);
}

fn describe_hovered_object(
    picking: &mut StaticPickingResources,
    cursor_pos: Option<Vec2>,
    camera: Option<(&Camera, &GlobalTransform)>,
    settings: &crate::configs::settings::Settings,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    map_id: u8,
) -> String {
    let Some(cursor_pos) = cursor_pos else {
        return "hovered_object: [no cursor]".to_string();
    };
    let Some((camera, camera_tf)) = camera else {
        return "hovered_object: [no camera]".to_string();
    };

    let Some(best_match) = find_hovered_static_object(
        picking,
        cursor_pos,
        camera,
        camera_tf,
        settings,
        tilemeta_res,
        map_id,
    ) else {
        return "hovered_object: none".to_string();
    };

    format!(
        "hovered_object: xy=[{}, {}] {}",
        best_match.global_x,
        best_match.global_y,
        describe_static_tile_details(
            tex_art_cc_res,
            tex_art_ec_res,
            tex_land_ec_res,
            tilemeta_res,
            best_match.tile,
            Some(best_match.kind),
            Some(best_match.route),
            picking.art_source(),
        )
    )
}

fn find_hovered_static_object(
    picking: &mut StaticPickingResources,
    cursor_pos: Vec2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    settings: &crate::configs::settings::Settings,
    tilemeta_res: Option<&TileMetaPackageRes>,
    map_id: u8,
) -> Option<HoveredStaticMatch> {
    if !settings.world_rendering.enable_statics { return None; }
    picking.prepare_alpha();
    let mut best_match: Option<HoveredStaticMatch> = None;

    for tile in picking.visible_tiles(u32::from(map_id)) {
        let Some(candidate) = hovered_static_match(picking, cursor_pos, camera,
            camera_tf, tilemeta_res, tile) else { continue; };
        if best_match.as_ref().is_none_or(|current| candidate.depth_key >= current.depth_key) {
            best_match = Some(candidate);
        }
    }

    best_match
}

fn hovered_static_match(
    picking: &mut StaticPickingResources,
    cursor_pos: Vec2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    tilemeta_res: Option<&TileMetaPackageRes>,
    candidate: crate::core::render::scene::world::art::statics_collect::StaticPickTile,
) -> Option<HoveredStaticMatch> {
    let source = candidate.tile;
    let tile = uocf::classic::statics::PackedStaticTile { graphic: source.graphic,
        xy_packed: (source.world_x as u32 & 7) as u8 | (((source.world_z as u32 & 7) as u8) << 3),
        z: source.z, hue: source.hue };
    let graphic = tile.graphic;
    let tilemeta = tilemeta_res.and_then(|meta| meta.0.item_tile(graphic as u32));
    if tilemeta.is_some_and(|meta| meta.is_nodraw()) {
        return None;
    }
    let depth_class = resolve_static_depth_class(tilemeta, candidate.is_wet);
    let world_x = source.world_x;
    let world_z = source.world_z;
    let geometry = picking.resolve(graphic, world_x, world_z, tile.z)?;
    let kind = if geometry.flat_quad == 0 { HoveredObjectKind::Sprite } else { HoveredObjectKind::Ground };
    let corners = geometry.corners(camera, camera_tf);

    let hit = projected_quad_hit(cursor_pos, camera, camera_tf, corners)?;
    if !picking.alpha_visible(geometry, hit, candidate.is_wet) { return None; }

    let priority_z_units = resolve_priority_z_units(tile.z, tilemeta, depth_class);
    let depth_key = static_depth_key(geometry.priority_anchor.x, geometry.priority_anchor.y,
        priority_z_units, depth_class);
    Some(HoveredStaticMatch {
        tile,
        kind,
        corners,
        depth_key,
        global_x: world_x as u16,
        global_y: world_z as u16,
        route: geometry.route,
    })
}

fn resolve_cursor_tile_coords(
    cursor_pos: Option<Vec2>,
    camera: Option<(&Camera, &GlobalTransform)>,
    player: Option<&Player>,
    settings: &crate::configs::settings::Settings,
) -> Option<(u8, u16, u16)> {
    let cursor_pos = cursor_pos?;
    let (camera, camera_tf) = camera?;
    let player = player?;
    let ray = camera.viewport_to_world(camera_tf, cursor_pos).ok()?;
    if ray.direction.y.abs() <= 1e-6 {
        return None;
    }

    let t = -ray.origin.y / ray.direction.y;
    let hit = ray.origin + ray.direction * t;
    let cursor_x = hit.x.round().max(0.0) as u16;
    let cursor_y = hit.z.round().max(0.0) as u16;
    let map_id = player
        .current_pos
        .map(|p| p.m)
        .unwrap_or(settings.session_state.world.last_p.m);
    Some((map_id, cursor_x, cursor_y))
}

fn describe_land_tile(
    map_planes_r: &MapPlanesRes,
    tilemeta_res: Option<&TileMetaPackageRes>,
    map_id: u8,
    x: u16,
    y: u16,
) -> String {
    let Some(plane) = map_planes_r
        .0
        .get(map_id as usize)
        .and_then(|opt| opt.as_ref())
    else {
        return "land: [missing map plane]".to_string();
    };

    let cell = MapCellCoords {
        x: x as u32,
        y: y as u32,
    };
    let block_pos = MapCell::coords_of_parent_block(&cell);
    let Some(block) = plane.block_no_update(block_pos) else {
        return "land: [uncached block]".to_string();
    };
    let rel = MapCell::coords_in_block(&cell);
    let Ok(cell) = block.cell(rel.x, rel.y) else {
        return "land: [invalid cell]".to_string();
    };

    let meta = tilemeta_res.and_then(|meta| meta.0.land_tile(cell.id as u32));
    let name = meta
        .map(|tile| tile.name_ascii())
        .filter(|name| !name.is_empty())
        .unwrap_or("<unnamed>");
    let texture_id = meta
        .map(|tile| tile.texture_id.to_string())
        .unwrap_or_else(|| "?".to_string());

    format!(
        "land: id={} z={} tex={} name={}",
        cell.id, cell.z, texture_id, name
    )
}

fn describe_static_tiles(
    picking: &mut StaticPickingResources,
    statics_res: &StaticsStoreRes,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    map_id: u8,
    x: u16,
    y: u16,
) -> String {
    let Some(store) = statics_res
        .0
        .get(map_id as usize)
        .and_then(|opt| opt.as_ref())
    else {
        return "statics: [missing store]".to_string();
    };

    let mut store = store.lock();
    let block_x = x as u32 / 8;
    let block_y = y as u32 / 8;
    let local_x = (x & 0x07) as u8;
    let local_y = (y & 0x07) as u8;
    let Ok(block_tiles) = store.block_tiles(block_x, block_y) else {
        return "statics: [read error]".to_string();
    };

    let mut matches = block_tiles
        .iter()
        .copied()
        .filter(|tile| tile.x_offset() == local_x && tile.y_offset() == local_y)
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return "statics: none".to_string();
    }

    matches.sort_unstable_by_key(|tile| std::cmp::Reverse(tile.z));

    let summary = matches
        .iter()
        .take(3)
        .map(|tile| {
            describe_static_tile_details(
                tex_art_cc_res,
                tex_art_ec_res,
                tex_land_ec_res,
                tilemeta_res,
                *tile,
                None,
                picking.resolve(tile.graphic, f32::from(x), f32::from(y), tile.z).map(|geometry| geometry.route),
                picking.art_source(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    if matches.len() > 3 {
        format!("statics: count={} top=\n{}\n...", matches.len(), summary)
    } else {
        format!("statics: count={}\n{}", matches.len(), summary)
    }
}

fn describe_static_tile_details(
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    tile: uocf::classic::statics::PackedStaticTile,
    hovered_kind: Option<HoveredObjectKind>,
    route: Option<StaticArtRoute>,
    art_source: Option<crate::configs::settings::ClientTextureSource>,
) -> String {
    let graphic = tile.graphic;
    let z = tile.z;
    let hue = tile.hue;
    let meta = tilemeta_res.and_then(|meta| meta.0.item_tile(graphic as u32));
    let name = meta
        .map(|item| item.name_ascii())
        .filter(|name| !name.is_empty())
        .unwrap_or("<unnamed>");
    let cc_texture_id = meta.map(|item| item.cc_texture_id);
    let (ec_texture_id, ec_tex_branch) = tilemeta_res
        .and_then(|meta_package| meta_package.0.main_ec_texture_id_with_branch(graphic as u32))
        .map(|(id, branch)| (Some(id), branch))
        .unwrap_or((None, "none"));
    let selected_source = match route {
        Some(StaticArtRoute::CcArt(_)) => Some(crate::configs::settings::ClientTextureSource::Cc),
        _ => art_source,
    };
    let renders_wet = !matches!(route, Some(StaticArtRoute::MissingArt)) &&
        static_tile_renders_wet(meta, tilemeta_res.map(|package| package.0.as_ref()), u32::from(graphic), selected_source);
    let is_surface_like = static_tile_is_surface_like(meta, renders_wet);
    let tex_art_cc_id = cc_texture_id.map(|id| id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET as u32));
    let cc_slot =
        tex_art_cc_id.and_then(|art_id| tex_art_cc_res.and_then(|package| package.0.present_slot(art_id)));
    let tex_art_ec_slot = tex_art_ec_res.and_then(|package| package.0.present_slot(graphic as u32));
    let tex_land_ec_runtime_slot = match route {
        Some(StaticArtRoute::EcLand(slot)) => Some(slot),
        _ => None,
    };
    let should_resolve_tex_land_ec_slot = tex_land_ec_runtime_slot.is_some();
    let tex_land_ec_slot = tex_land_ec_runtime_slot
        .and_then(|slot_id| tex_land_ec_res.and_then(|package| package.0.present_slot(slot_id)));
    let tex_land_ec_direct_slot = tex_land_ec_res.and_then(|p| p.0.present_slot(graphic as u32));
    let live_decision = match route {
        Some(StaticArtRoute::CcArt(slot)) => format!("cc->art:{}", slot),
        Some(StaticArtRoute::EcArt(slot)) => format!("ec->art:{}", slot),
        Some(StaticArtRoute::MissingArt) => "ec->missing.tga".into(),
        Some(StaticArtRoute::EcLand(slot)) => format!("ec->land:{}", slot),
        None => "unresolved/not_rendered".to_string(),
    };
    let hovered_kind_label = match hovered_kind {
        Some(HoveredObjectKind::Sprite) => " kind=sprite",
        Some(HoveredObjectKind::Ground) => " kind=ground",
        None => "",
    };

    // Split across several shorter lines instead of one long one: a single-line dump of
    // every diagnostic field ran off the edge of the screen and was unreadable (this is a
    // debug/dev overlay, so we keep every field, just wrapped instead of truncated).
    format!(
        "  id={} z={} hue={} name={} surf={}\n  cc_tex={} ec_tex={} ec_tex_branch={}\n  cc_slot={} tex_art_ec_slot={}\n  tex_land_ec_slot={} tex_land_ec_direct={}\n  live={}{}",
        graphic,
        z,
        hue,
        name,
        is_surface_like,
        optional_u32(cc_texture_id),
        optional_u32(ec_texture_id),
        ec_tex_branch,
        tex_art_cc_id
            .map(|id| format!("{}:{}", id, slot_presence(cc_slot.is_some())))
            .unwrap_or_else(|| "?".to_string()),
        slot_record_summary_u32(graphic as u32, tex_art_ec_slot.map(|slot| slot.page_index)),
        if should_resolve_tex_land_ec_slot {
            tex_land_ec_runtime_slot
                .map(|runtime_slot| {
                    format!(
                        "runtime:{} {}",
                        runtime_slot,
                        slot_presence(tex_land_ec_slot.is_some())
                    )
                })
                .unwrap_or_else(|| "unresolved".to_string())
        } else {
            "not_checked".to_string()
        },
        tex_land_ec_direct_slot
            .map(|slot| slot_record_summary_u32(graphic as u32, Some(slot.page_index)))
            .unwrap_or_else(|| "absent".to_string()),
        live_decision,
        hovered_kind_label,
    )
}

fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn slot_presence(is_present: bool) -> &'static str {
    if is_present {
        "present"
    } else {
        "missing"
    }
}

fn slot_record_summary_u32(art_id: u32, page_index: Option<u32>) -> String {
    match page_index {
        Some(page_index) => format!("{}:present@p{}", art_id, page_index),
        None => format!("{}:missing", art_id),
    }
}

fn sys_teleport_on_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
    cursor: Res<CursorBehavior>,
    settings: Res<crate::configs::settings::Settings>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    world_geo_data: Res<WorldGeoData>,
) {
    if cursor.mode != CursorMode::Teleport {
        return;
    }

    // If egui wants pointer input, don't teleport
    if let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) {
        if ctx.egui_wants_pointer_input() {
            return;
        }
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

    let Ok((cam, cam_tf)) = camera_q.single() else {
        return;
    };
    let ray = match cam.viewport_to_world(cam_tf, cursor_pos).ok() {
        Some(ray) => ray,
        None => return,
    };

    let dir_y = ray.direction.y;
    if dir_y.abs() <= 1e-6 {
        return;
    }

    let t = -ray.origin.y / dir_y;
    let hit = ray.origin + ray.direction * t;

    if let Ok((mut player, mut transform)) = player_q.single_mut() {
        let map = player
            .current_pos
            .map(|p| p.m)
            .unwrap_or(settings.session_state.world.last_p.m);

        let (max_x, max_y) = if let Some(meta) = world_geo_data.maps.get(&(map as u32)) {
            (meta.width, meta.height)
        } else {
            (u16::MAX as u32, u16::MAX as u32)
        };

        let mut clamped_hit = hit;
        clamped_hit.x = clamped_hit.x.clamp(0.0, (max_x.saturating_sub(1)) as f32);
        clamped_hit.z = clamped_hit.z.clamp(0.0, (max_y.saturating_sub(1)) as f32);

        let uo_pos = clamped_hit.to_uo_vec4(map);
        player.current_pos = Some(uo_pos);
        transform.translation = uo_pos.to_bevy_vec3_ignore_map();
        // Always force a full recompute so the new visible area loads immediately.
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});

        ingame_sysmessage_logger::normal(format!(
            "Teleported to [{}, {}, {}, {}] via cursor",
            uo_pos.x, uo_pos.y, uo_pos.z, uo_pos.m
        ));
    }
}
