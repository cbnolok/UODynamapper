use crate::core::controls::input_actions::{
    ActionToggleCursorInspectPanel, ActionToggleCursorTeleportMode,
};
use crate::core::render::scene::world::art::statics_collect::{
    depth_class_y_bias, resolve_priority_z_units, resolve_static_billboard_bounds,
    resolve_static_depth_class, resolve_surface_like_ground_quad_bounds, static_depth_key,
    static_tile_is_surface_like,
};
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
use std::collections::BTreeSet;
use uocf::classic::map::{MapCell, MapCellCoords};

const FONT_SIZE: f32 = 13.0;
const CLASSIC_STATIC_ART_ID_OFFSET: u16 = 0x4000;
const INV_SQRT_2: f32 = 0.70710678118;
const BILLBOARD_RIGHT_XZ: Vec2 = Vec2::new(INV_SQRT_2, -INV_SQRT_2);
const HOVERED_STATIC_HIGHLIGHT_Y_LIFT: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMode {
    Select,
    Teleport,
}

impl Default for CursorMode {
    fn default() -> Self {
        CursorMode::Select
    }
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
    last_camera_translation: Local<'s, Option<Vec3>>,
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
                    update_cursor_inspect_text.run_if(in_state(AppState::InGame)),
                    sys_draw_hovered_static_highlight.run_if(in_state(AppState::InGame)),
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
                display: if settings.app.performance.show_overlay {
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
                    font: font.clone(),
                    font_size: FONT_SIZE * scale,
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
                    font: font.clone(),
                    font_size: FONT_SIZE * scale,
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
                            font: font.clone(),
                            font_size: FONT_SIZE * scale,
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
    windows: Query<&Window, With<PrimaryWindow>>,
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
    let current_scale = settings.app.window.cursor_position_scale;
    let scale_changed = (*last_scale - current_scale).abs() > 0.001;

    let show_overlay = settings.app.performance.show_overlay;
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
            text_font.font_size = FONT_SIZE * current_scale;
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
        }
        if let Ok((_, mut text_font, mut line_height)) = position_text_q.single_mut() {
            text_font.font_size = FONT_SIZE * current_scale;
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
        .and_then(|window| window.cursor_position());
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

pub fn update_cursor_inspect_text(
    resources: CursorInspectResources,
    player_q: Query<&Player>,
    windows: Query<&Window, With<PrimaryWindow>>,
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
        resources.settings.app.performance.show_overlay && resources.inspect_state.enabled;

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
            text_font.font_size = FONT_SIZE * current_scale;
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
        }
        *locals.last_scale = current_scale;
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
        .and_then(|window| window.cursor_position());
    let player_map_id = player_q
        .single()
        .ok()
        .and_then(|player| player.current_pos.map(|p| p.m));

    let needs_rebuild = *locals.last_cursor_pos != cursor_pos
        || *locals.last_window_size != window_size
        || *locals.last_camera_translation != camera_translation
        || *locals.last_player_map_id != player_map_id
        || scale_changed
        || resources.inspect_state.is_changed();

    if !show_inspect || !needs_rebuild {
        return;
    }

    let inspect_label = build_cursor_inspect_label(
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
    *locals.last_camera_translation = camera_translation;
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
        cursor_pos,
        camera,
        settings,
        statics_res,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
        map_id,
        cursor_x,
        cursor_y,
    );
    let land_line = describe_land_tile(map_planes_r, tilemeta_res, map_id, cursor_x, cursor_y);
    let statics_line = describe_static_tiles(
        settings,
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
}

fn sys_draw_hovered_static_highlight(
    settings: Res<crate::configs::settings::Settings>,
    player_q: Query<&Player>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    statics_res: Res<StaticsStoreRes>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
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

    let Some(cursor_pos) = windows.single().ok().and_then(|window| window.cursor_position()) else {
        return;
    };
    let Some((camera, camera_tf)) = camera_q.single().ok() else {
        return;
    };
    let Some(player) = player_q.single().ok() else {
        return;
    };
    let Some((map_id, x, y)) = resolve_cursor_tile_coords(
        Some(cursor_pos),
        Some((camera, camera_tf)),
        Some(player),
        &settings,
    ) else {
        return;
    };

    let Some(best_match) = find_hovered_static_object(
        cursor_pos,
        camera,
        camera_tf,
        &settings,
        &statics_res,
        tex_art_cc_res.as_deref(),
        tex_art_ec_res.as_deref(),
        tex_land_ec_res.as_deref(),
        tilemeta_res.as_deref(),
        map_id,
        x,
        y,
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
    cursor_pos: Option<Vec2>,
    camera: Option<(&Camera, &GlobalTransform)>,
    settings: &crate::configs::settings::Settings,
    statics_res: &StaticsStoreRes,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    map_id: u8,
    x: u16,
    y: u16,
) -> String {
    let Some(cursor_pos) = cursor_pos else {
        return "hovered_object: [no cursor]".to_string();
    };
    let Some((camera, camera_tf)) = camera else {
        return "hovered_object: [no camera]".to_string();
    };

    let Some(best_match) = find_hovered_static_object(
        cursor_pos,
        camera,
        camera_tf,
        settings,
        statics_res,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
        map_id,
        x,
        y,
    ) else {
        return "hovered_object: none".to_string();
    };

    format!(
        "hovered_object: xy=[{}, {}] {}",
        best_match.global_x,
        best_match.global_y,
        describe_static_tile_details(
            settings,
            tex_art_cc_res,
            tex_art_ec_res,
            tex_land_ec_res,
            tilemeta_res,
            best_match.tile,
            Some(best_match.kind),
        )
    )
}

fn find_hovered_static_object(
    cursor_pos: Vec2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    settings: &crate::configs::settings::Settings,
    statics_res: &StaticsStoreRes,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    map_id: u8,
    x: u16,
    y: u16,
) -> Option<HoveredStaticMatch> {
    let store = statics_res
        .0
        .get(map_id as usize)
        .and_then(|opt| opt.as_ref())?;
    let mut store = store.lock();
    let base_block_x = x as i32 / 8;
    let base_block_y = y as i32 / 8;
    let mut best_match: Option<HoveredStaticMatch> = None;

    for block_x in (base_block_x - 1)..=(base_block_x + 1) {
        for block_y in (base_block_y - 1)..=(base_block_y + 1) {
            if block_x < 0 || block_y < 0 {
                continue;
            }

            let Ok(block_tiles) = store.block_tiles(block_x as u32, block_y as u32) else {
                continue;
            };

            for tile in block_tiles.iter().copied() {
                let Some(candidate) = hovered_static_match(
                    cursor_pos,
                    camera,
                    camera_tf,
                    settings,
                    tex_art_cc_res,
                    tex_art_ec_res,
                    tex_land_ec_res,
                    tilemeta_res,
                    block_x as u32,
                    block_y as u32,
                    tile,
                ) else {
                    continue;
                };

                let should_replace = match &best_match {
                    Some(current) => candidate.depth_key >= current.depth_key,
                    None => true,
                };

                if should_replace {
                    best_match = Some(candidate);
                }
            }
        }
    }

    best_match
}

fn hovered_static_match(
    cursor_pos: Vec2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    settings: &crate::configs::settings::Settings,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    block_x: u32,
    block_y: u32,
    tile: uocf::classic::statics::PackedStaticTile,
) -> Option<HoveredStaticMatch> {
    let graphic = tile.graphic;
    let tilemeta = tilemeta_res.and_then(|meta| meta.0.item_tile(graphic as u32));
    let depth_class = resolve_static_depth_class(tilemeta);
    let local_x = tile.x_offset() as f32;
    let local_y = tile.y_offset() as f32;
    let world_x = block_x as f32 * 8.0 + local_x;
    let world_z = block_y as f32 * 8.0 + local_y;
    let world_y = tile.z as f32 * 0.1 + depth_class_y_bias(depth_class);
    let (kind, corners) = match resolve_hovered_static_geometry(
        settings,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
        tilemeta,
        graphic,
        world_x,
        world_y,
        world_z,
    ) {
        Some(value) => value,
        None => return None,
    };

    if !screen_polygon_contains(cursor_pos, camera, camera_tf, &corners) {
        return None;
    }

    let priority_z_units = resolve_priority_z_units(tile.z, tilemeta, depth_class);
    let depth_key = static_depth_key(world_x, world_z, priority_z_units, depth_class);
    Some(HoveredStaticMatch {
        tile,
        kind,
        corners,
        depth_key,
        global_x: world_x as u16,
        global_y: world_z as u16,
    })
}

fn resolve_hovered_static_geometry(
    settings: &crate::configs::settings::Settings,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    graphic: u16,
    world_x: f32,
    world_y: f32,
    world_z: f32,
) -> Option<(HoveredObjectKind, [Vec3; 4])> {
    match settings.graphics.art_texture_source {
        crate::configs::settings::ClientTextureSource::Cc => {
            let world_x = world_x + 0.5;
            let world_z = world_z + 1.5;
            let cc_texture_id = tilemeta
                .map(|meta| meta.cc_texture_id as u16)
                .unwrap_or(graphic);
            let art_id = cc_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET);
            let slot = tex_art_cc_res?.0.present_slot(art_id as u32)?;
            let bounds = resolve_static_billboard_bounds(
                crate::configs::settings::ClientTextureSource::Cc,
                slot.draw_offset_x,
                slot.draw_offset_y,
                slot.logical_width(),
                slot.logical_height(),
            );
            Some((
                HoveredObjectKind::Sprite,
                billboard_corners(
                    world_x,
                    world_y,
                    world_z,
                    bounds.local_min_x,
                    bounds.local_max_x,
                    bounds.local_min_y,
                    bounds.local_max_y,
                ),
            ))
        }
        crate::configs::settings::ClientTextureSource::Ec => {
            let world_x = world_x + 0.5;
            let world_z = world_z + 1.5;
            if let Some(runtime_slot_id) = resolve_overlay_tex_land_ec_runtime_slot(
                graphic as u32,
                tilemeta_res.map(|package| &*package.0),
                tilemeta,
                tex_land_ec_res.map(|package| &*package.0),
            ) {
                if tex_land_ec_res
                    .and_then(|package| (&*package.0).present_slot(runtime_slot_id))
                    .is_some()
                {
                    let bounds = resolve_surface_like_ground_quad_bounds();
                    return Some((
                        HoveredObjectKind::Ground,
                        [
                            Vec3::new(
                                world_x + bounds.local_min_x,
                                world_y,
                                world_z + bounds.local_min_z,
                            ),
                            Vec3::new(
                                world_x + bounds.local_max_x,
                                world_y,
                                world_z + bounds.local_min_z,
                            ),
                            Vec3::new(
                                world_x + bounds.local_min_x,
                                world_y,
                                world_z + bounds.local_max_z,
                            ),
                            Vec3::new(
                                world_x + bounds.local_max_x,
                                world_y,
                                world_z + bounds.local_max_z,
                            ),
                        ],
                    ));
                }
            }

            let slot = tex_art_ec_res?.0.present_slot(graphic as u32)?;
            let bounds = resolve_static_billboard_bounds(
                crate::configs::settings::ClientTextureSource::Ec,
                slot.draw_offset_x,
                slot.draw_offset_y,
                slot.logical_width(),
                slot.logical_height(),
            );
            Some((
                HoveredObjectKind::Sprite,
                billboard_corners(
                    world_x,
                    world_y,
                    world_z,
                    bounds.local_min_x,
                    bounds.local_max_x,
                    bounds.local_min_y,
                    bounds.local_max_y,
                ),
            ))
        }
    }
}

fn billboard_corners(
    world_x: f32,
    world_y: f32,
    world_z: f32,
    local_min_x: f32,
    local_max_x: f32,
    local_min_y: f32,
    local_max_y: f32,
) -> [Vec3; 4] {
    [
        billboard_corner(world_x, world_y, world_z, local_min_x, local_max_y),
        billboard_corner(world_x, world_y, world_z, local_max_x, local_max_y),
        billboard_corner(world_x, world_y, world_z, local_min_x, local_min_y),
        billboard_corner(world_x, world_y, world_z, local_max_x, local_min_y),
    ]
}

fn billboard_corner(world_x: f32, world_y: f32, world_z: f32, local_x: f32, local_y: f32) -> Vec3 {
    Vec3::new(
        world_x + local_x * BILLBOARD_RIGHT_XZ.x,
        world_y + local_y,
        world_z + local_x * BILLBOARD_RIGHT_XZ.y,
    )
}

fn screen_polygon_contains(
    cursor_pos: Vec2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    corners: &[Vec3; 4],
) -> bool {
    let mut screen_points = Vec::with_capacity(corners.len());
    for corner in corners {
        let Ok(screen) = camera.world_to_viewport(camera_tf, *corner) else {
            return false;
        };
        screen_points.push(screen);
    }

    let min_x = screen_points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = screen_points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = screen_points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = screen_points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);

    cursor_pos.x >= min_x && cursor_pos.x <= max_x && cursor_pos.y >= min_y && cursor_pos.y <= max_y
}

fn resolve_overlay_tex_land_ec_runtime_slot(
    tile_id: u32,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> Option<u32> {
    let Some(meta) = tilemeta else {
        return None;
    };
    if !static_tile_is_surface_like(tilemeta) {
        return None;
    }

    let Some(package) = tex_land_ec else {
        return None;
    };

    let Some(main_ec_texture_id) = tilemeta_package
        .and_then(|package| package.main_ec_texture_id(tile_id))
        .or_else(|| (meta.ec_texture_id != 0).then_some(meta.ec_texture_id))
    else {
        return package.resolve_runtime_slot_id(meta.cc_texture_id);
    };

    if package.present_slot(main_ec_texture_id).is_some() {
        return Some(main_ec_texture_id);
    }

    let mut canonical_slots = BTreeSet::new();
    let mut alias_slots = BTreeSet::new();
    for record in package
        .terrain_provenance()
        .iter()
        .filter(|record| record.selected_texture_id == main_ec_texture_id)
    {
        if record.canonical_slot_id != 0
            && record.canonical_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.canonical_slot_id).is_some()
        {
            canonical_slots.insert(record.canonical_slot_id);
        }

        if record.alias_slot_id != 0
            && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.alias_slot_id).is_some()
        {
            alias_slots.insert(record.alias_slot_id);
        }
    }

    if canonical_slots.len() == 1 {
        return canonical_slots.into_iter().next();
    }

    if canonical_slots.is_empty() && alias_slots.len() == 1 {
        return alias_slots.into_iter().next();
    }

    package.resolve_runtime_slot_id(meta.cc_texture_id)
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
    settings: &crate::configs::settings::Settings,
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

    matches.sort_unstable_by(|left, right| right.z.cmp(&left.z));

    let summary = matches
        .iter()
        .take(3)
        .map(|tile| {
            describe_static_tile_details(
                settings,
                tex_art_cc_res,
                tex_art_ec_res,
                tex_land_ec_res,
                tilemeta_res,
                *tile,
                None,
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");

    if matches.len() > 3 {
        format!("statics: count={} top={} ...", matches.len(), summary)
    } else {
        format!("statics: count={} {}", matches.len(), summary)
    }
}

fn describe_static_tile_details(
    settings: &crate::configs::settings::Settings,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    tile: uocf::classic::statics::PackedStaticTile,
    hovered_kind: Option<HoveredObjectKind>,
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
    let ec_texture_id = tilemeta_res.and_then(|meta_package| meta_package.0.main_ec_texture_id(graphic as u32));
    let is_surface_like = static_tile_is_surface_like(meta);
    let tex_art_cc_id = cc_texture_id.map(|id| id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET as u32));
    let cc_slot =
        tex_art_cc_id.and_then(|art_id| tex_art_cc_res.and_then(|package| package.0.present_slot(art_id)));
    let tex_art_ec_slot = tex_art_ec_res.and_then(|package| package.0.present_slot(graphic as u32));
    let tex_land_ec_runtime_slot = resolve_overlay_tex_land_ec_runtime_slot(
        graphic as u32,
        tilemeta_res.map(|meta_package| &*meta_package.0),
        meta,
        tex_land_ec_res.map(|res| &*res.0),
    );
    let tex_land_ec_slot = tex_land_ec_runtime_slot
        .and_then(|slot_id| tex_land_ec_res.and_then(|package| (&*package.0).present_slot(slot_id)));
    let live_decision = match settings.graphics.art_texture_source {
        crate::configs::settings::ClientTextureSource::Cc => {
            format!(
                "cc->art:{} {}",
                tex_art_cc_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                slot_presence(cc_slot.is_some())
            )
        }
        crate::configs::settings::ClientTextureSource::Ec => {
            if tex_land_ec_runtime_slot.is_some() {
                format!(
                    "ec->land:{} {}",
                    tex_land_ec_runtime_slot
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "unresolved".to_string()),
                    slot_presence(tex_land_ec_slot.is_some())
                )
            } else {
                format!(
                    "ec->art:{} {}",
                    graphic,
                    slot_presence(tex_art_ec_slot.is_some())
                )
            }
        }
    };
    let hovered_kind_label = match hovered_kind {
        Some(HoveredObjectKind::Sprite) => " kind=sprite",
        Some(HoveredObjectKind::Ground) => " kind=ground",
        None => "",
    };

    format!(
        "id={} z={} hue={} name={} surf={} cc_tex={} ec_tex={} cc_slot={} tex_art_ec_slot={} tex_land_ec_slot={} {}{}",
        graphic,
        z,
        hue,
        name,
        is_surface_like,
        optional_u32(cc_texture_id),
        optional_u32(ec_texture_id),
        tex_art_cc_id
            .map(|id| format!("{}:{}", id, slot_presence(cc_slot.is_some())))
            .unwrap_or_else(|| "?".to_string()),
        slot_record_summary_u32(graphic as u32, tex_art_ec_slot.map(|slot| slot.page_index)),
        tex_land_ec_runtime_slot
            .map(|runtime_slot| {
                format!("runtime:{} {}", runtime_slot, slot_presence(tex_land_ec_slot.is_some()))
            })
            .unwrap_or_else(|| "unresolved".to_string()),
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
        if ctx.wants_pointer_input() {
            return;
        }
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(window) = windows.single().ok() else {
        return;
    };

    let cursor_pos = match window.cursor_position() {
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
