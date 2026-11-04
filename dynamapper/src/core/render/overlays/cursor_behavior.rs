use crate::core::controls::input_actions::{
    ActionToggleCursorInspectPanel, ActionToggleCursorTeleportMode,
};
use crate::core::controls::{InjectedPointer, effective_cursor_position};
use crate::core::render::scene::world::art::statics_collect::{
    static_tile_is_surface_like, static_tile_renders_wet,
};
use crate::core::render::scene::world::art::static_geometry::{StaticArtRoute, StaticPickingResources};
use crate::core::render::scene::player::Player;
use crate::core::render::scene::world::land::picking::{TerrainCursor, TerrainCursorQuery};
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
// The inspector stays a compact panel in the corner: long values wrap inside this width.
const CURSOR_INSPECT_MAX_WIDTH_PX: f32 = 400.0;
const CURSOR_INSPECT_MAX_STATICS: usize = 3;
const CURSOR_INSPECT_NAME_CHARS: usize = 20;
const CURSOR_INSPECT_NAME_ELLIPSIS: &str = "..";

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
    terrain: Res<'w, TerrainCursor>,
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
                    update_cursor_behavior_text.after(TerrainCursorQuery).run_if(in_state(AppState::InGame)),
                    update_cursor_inspect_text.after(TerrainCursorQuery).after(crate::core::system_sets::SceneRenderArtSysSet::CollectVisibleStatics)
                        .run_if(in_state(AppState::InGame)),
                    sys_draw_hovered_static_highlight.after(crate::core::system_sets::SceneRenderArtSysSet::CollectVisibleStatics)
                        .run_if(in_state(AppState::InGame)),
                    sys_teleport_on_click.after(TerrainCursorQuery).run_if(in_state(AppState::InGame)),
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
                max_width: Val::Px(CURSOR_INSPECT_MAX_WIDTH_PX * settings.app.window.cursor_position_scale),
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
                        Text::new("Cell: [disabled]"),
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

pub(crate) fn update_cursor_behavior_text(
    settings: Res<crate::configs::settings::Settings>,
    cursor: Res<CursorBehavior>,
    player_q: Query<&Player>,
    windows: (Query<&Window, With<PrimaryWindow>>, Option<Res<InjectedPointer>>),
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    terrain: Res<TerrainCursor>,
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
        || *last_player_map_id != player_map_id
        || terrain.is_changed();

    if needs_rebuild {
        let cursor_position_label = match terrain.hit {
            Some(hit) => format!("Cursor position:\n[{}, {}, {}]", hit.logical.x, hit.logical.y, hit.logical.z),
            None => "Cursor position:\n[NA, NA, NA]".to_string(),
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
        || picking.is_changed()
        || resources.terrain.is_changed();

    if !show_inspect || !needs_rebuild {
        return;
    }

    let inspect_label = build_cursor_inspect_label(
        &mut picking,
        cursor_pos,
        camera_q.single().ok(),
        player_q.single().ok(),
        &resources.settings,
        &resources.terrain,
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
    terrain: &TerrainCursor,
    map_planes_r: &MapPlanesRes,
    statics_res: &StaticsStoreRes,
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
) -> String {
    let Some(map_id) = cursor_map_id(player, settings) else {
        return "Cell: [NA]".to_string();
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
    let Some(hit) = terrain.hit.filter(|hit| hit.logical.m == map_id) else {
        return format!("Cell: no terrain, map {}\n{}", map_id, hovered_object_line);
    };
    let (cursor_x, cursor_y) = (hit.logical.x, hit.logical.y);
    let land_line = describe_land_tile(map_planes_r, tilemeta_res, map_id, cursor_x, cursor_y);
    let statics_line = describe_static_tiles(
        picking,
        statics_res,
        tilemeta_res,
        map_id,
        cursor_x,
        cursor_y,
    );

    format!(
        "Cell: {},{} map {} art {:?}\n{}\n{}\n{}",
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
    let Some(map_id) = cursor_map_id(Some(player), &settings) else { return; };

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
        return "Object: [no cursor]".to_string();
    };
    let Some((camera, camera_tf)) = camera else {
        return "Object: [no camera]".to_string();
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
        return "Object: none".to_string();
    };

    let kind = match best_match.kind {
        HoveredObjectKind::Sprite => "sprite",
        HoveredObjectKind::Ground => "ground",
    };
    format!(
        "Object: {}\n  at {},{} {}\n  {}",
        describe_static_tile_line(tilemeta_res, best_match.tile, Some(best_match.route), picking.art_source()),
        best_match.global_x,
        best_match.global_y,
        kind,
        describe_static_tile_sources(
            tex_art_cc_res,
            tex_art_ec_res,
            tex_land_ec_res,
            tilemeta_res,
            best_match.tile,
            Some(best_match.route),
        ),
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
    let tile = picking.visible_hit(cursor_pos, camera, camera_tf, u32::from(map_id))?;
    hovered_static_match(picking, camera, camera_tf, tilemeta_res, tile)
}

fn hovered_static_match(
    picking: &mut StaticPickingResources,
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
    let world_x = source.world_x;
    let world_z = source.world_z;
    let geometry = picking.resolve(graphic, world_x, world_z, tile.z)?;
    let kind = if geometry.flat_quad == 0 { HoveredObjectKind::Sprite } else { HoveredObjectKind::Ground };
    let corners = geometry.corners(camera, camera_tf);

    // Visibility is the GPU material/depth result. Reconstruct only the logical
    // owner and its highlight; do not re-reject it with a different CPU sampler.

    Some(HoveredStaticMatch {
        tile,
        kind,
        corners,
        global_x: world_x as u16,
        global_y: world_z as u16,
        route: geometry.route,
    })
}

fn cursor_map_id(player: Option<&Player>, settings: &crate::configs::settings::Settings) -> Option<u8> {
    Some(player?.current_pos.map(|p| p.m).unwrap_or(settings.session_state.world.last_p.m))
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
        return "Land: [missing map plane]".to_string();
    };

    let cell = MapCellCoords {
        x: x as u32,
        y: y as u32,
    };
    let block_pos = MapCell::coords_of_parent_block(&cell);
    let Some(block) = plane.block_no_update(block_pos) else {
        return "Land: [uncached block]".to_string();
    };
    let rel = MapCell::coords_in_block(&cell);
    let Ok(cell) = block.cell(rel.x, rel.y) else {
        return "Land: [invalid cell]".to_string();
    };

    let meta = tilemeta_res.and_then(|meta| meta.0.land_tile(cell.id as u32));
    let name = meta
        .map(|tile| tile.name_ascii())
        .filter(|name| !name.is_empty())
        .unwrap_or("<unnamed>");
    let texture_id = meta
        .map(|tile| tile.texture_id.to_string())
        .unwrap_or_else(|| "?".to_string());

    format!("Land: {} z{} tex {} {}", cell.id, cell.z, texture_id, shorten_tile_name(name))
}

fn describe_static_tiles(
    picking: &mut StaticPickingResources,
    statics_res: &StaticsStoreRes,
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
        return "Statics: [missing store]".to_string();
    };

    let mut store = store.lock();
    let block_x = x as u32 / 8;
    let block_y = y as u32 / 8;
    let local_x = (x & 0x07) as u8;
    let local_y = (y & 0x07) as u8;
    let Ok(block_tiles) = store.block_tiles(block_x, block_y) else {
        return "Statics: [read error]".to_string();
    };

    let mut matches = block_tiles
        .iter()
        .copied()
        .filter(|tile| tile.x_offset() == local_x && tile.y_offset() == local_y)
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return "Statics: none".to_string();
    }

    matches.sort_unstable_by_key(|tile| std::cmp::Reverse(tile.z));

    let summary = matches
        .iter()
        .take(CURSOR_INSPECT_MAX_STATICS)
        .map(|tile| {
            let route = picking.resolve(tile.graphic, f32::from(x), f32::from(y), tile.z).map(|geometry| geometry.route);
            format!("  {}", describe_static_tile_line(tilemeta_res, *tile, route, picking.art_source()))
        })
        .collect::<Vec<_>>()
        .join("\n");

    let hidden = matches.len().saturating_sub(CURSOR_INSPECT_MAX_STATICS);
    if hidden > 0 {
        format!("Statics: {}\n{}\n  +{} more", matches.len(), summary, hidden)
    } else {
        format!("Statics: {}\n{}", matches.len(), summary)
    }
}

/// One line per tile: what it is and how it is drawn.
fn describe_static_tile_line(
    tilemeta_res: Option<&TileMetaPackageRes>,
    tile: uocf::classic::statics::PackedStaticTile,
    route: Option<StaticArtRoute>,
    art_source: Option<crate::configs::settings::ClientTextureSource>,
) -> String {
    let graphic = tile.graphic;
    let meta = tilemeta_res.and_then(|meta| meta.0.item_tile(graphic as u32));
    let name = meta
        .map(|item| item.name_ascii())
        .filter(|name| !name.is_empty())
        .unwrap_or("<unnamed>");
    let selected_source = match route {
        Some(StaticArtRoute::CcArt(_)) => Some(crate::configs::settings::ClientTextureSource::Cc),
        _ => art_source,
    };
    let renders_wet = !matches!(route, Some(StaticArtRoute::MissingArt)) &&
        static_tile_renders_wet(meta, tilemeta_res.map(|package| package.0.as_ref()), u32::from(graphic), selected_source);
    format_static_tile_line(
        graphic,
        tile.z,
        tile.hue,
        name,
        static_tile_is_surface_like(meta, renders_wet),
        route,
    )
}

fn format_static_tile_line(
    graphic: u16,
    z: i8,
    hue: u16,
    name: &str,
    is_surface_like: bool,
    route: Option<StaticArtRoute>,
) -> String {
    let mut line = format!("{} z{} {}", graphic, z, shorten_tile_name(name));
    if hue != 0 {
        line.push_str(&format!(" hue {}", hue));
    }
    if is_surface_like {
        line.push_str(" surface");
    }
    line.push_str(&format!(": {}", static_route_label(route)));
    line
}

fn static_route_label(route: Option<StaticArtRoute>) -> String {
    match route {
        Some(StaticArtRoute::CcArt(slot)) => format!("cc art {}", slot),
        Some(StaticArtRoute::EcArt(slot)) => format!("ec art {}", slot),
        Some(StaticArtRoute::MissingArt) => "missing art".to_string(),
        Some(StaticArtRoute::EcLand(slot)) => format!("ec land {}", slot),
        None => "not drawn".to_string(),
    }
}

fn shorten_tile_name(name: &str) -> String {
    match name.char_indices().nth(CURSOR_INSPECT_NAME_CHARS) {
        Some((end, _)) => format!("{}{}", &name[..end], CURSOR_INSPECT_NAME_ELLIPSIS),
        None => name.to_string(),
    }
}

/// The source textures of the hovered tile, naming only the package slots that are absent.
fn describe_static_tile_sources(
    tex_art_cc_res: Option<&TexArtCcPackageRes>,
    tex_art_ec_res: Option<&TexArtEcPackageRes>,
    tex_land_ec_res: Option<&TexLandEcPackageRes>,
    tilemeta_res: Option<&TileMetaPackageRes>,
    tile: uocf::classic::statics::PackedStaticTile,
    route: Option<StaticArtRoute>,
) -> String {
    let graphic = u32::from(tile.graphic);
    let cc_texture_id = tilemeta_res
        .and_then(|meta| meta.0.item_tile(graphic))
        .map(|item| item.cc_texture_id);
    let ec_texture = tilemeta_res.and_then(|meta| meta.0.main_ec_texture_id_with_branch(graphic));
    let cc_art_present = cc_texture_id
        .map(|id| id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET as u32))
        .and_then(|art_id| tex_art_cc_res.and_then(|package| package.0.present_slot(art_id)))
        .is_some();
    let ec_art_present = tex_art_ec_res.and_then(|package| package.0.present_slot(graphic)).is_some();
    let ec_land_present = match route {
        Some(StaticArtRoute::EcLand(slot)) => tex_land_ec_res.and_then(|package| package.0.present_slot(slot)).is_some(),
        _ => true,
    };

    let mut sources = format!("tex cc {}", cc_texture_id.map_or("?".to_string(), |id| id.to_string()));
    match ec_texture {
        Some((id, branch)) => sources.push_str(&format!(" ec {} ({})", id, branch)),
        None => sources.push_str(" ec ?"),
    }
    let missing: Vec<&str> = [
        (cc_art_present, "cc art"),
        (ec_art_present, "ec art"),
        (ec_land_present, "ec land"),
    ]
    .into_iter()
    .filter_map(|(present, label)| (!present).then_some(label))
    .collect();
    if !missing.is_empty() {
        sources.push_str(&format!("; no {}", missing.join(", ")));
    }
    sources
}

fn sys_teleport_on_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
    cursor: Res<CursorBehavior>,
    settings: Res<crate::configs::settings::Settings>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
    world_geo_data: Res<WorldGeoData>,
    terrain: Res<TerrainCursor>,
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

    let Some(hit) = terrain.hit.filter(|_| terrain.pointer == Some(cursor_pos)) else { return; };
    if let Ok((mut player, mut transform)) = player_q.single_mut() {
        let Some(map) = cursor_map_id(Some(&player), &settings) else { return; };
        if hit.logical.m != map { return; }
        if crate::core::controls::teleport::teleport_player(&mut player, &mut transform, hit.logical, &world_geo_data).is_err() { return; }
        // Always force a full recompute so the new visible area loads immediately.
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});
        ingame_sysmessage_logger::normal(format!(
            "Teleported to [{}, {}, {}, {}] via cursor",
            hit.logical.x, hit.logical.y, hit.logical.z, hit.logical.m
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_static_line_names_only_what_applies() {
        assert_eq!(
            format_static_tile_line(1826, 7, 0, "stone wall", false, Some(StaticArtRoute::CcArt(18210))),
            "1826 z7 stone wall: cc art 18210",
        );
        assert_eq!(
            format_static_tile_line(6039, -5, 33, "water", true, Some(StaticArtRoute::EcLand(12))),
            "6039 z-5 water hue 33 surface: ec land 12",
        );
        assert_eq!(format_static_tile_line(1, 0, 0, "x", false, None), "1 z0 x: not drawn");
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let long = "a very long tile name that would overflow";
        let shortened = shorten_tile_name(long);
        assert_eq!(shortened.chars().count(), CURSOR_INSPECT_NAME_CHARS + CURSOR_INSPECT_NAME_ELLIPSIS.len());
        assert!(shortened.ends_with(CURSOR_INSPECT_NAME_ELLIPSIS));
        assert_eq!(shorten_tile_name("short"), "short");
        assert_eq!(shorten_tile_name(&"\u{e8}".repeat(CURSOR_INSPECT_NAME_CHARS + 1)).chars().count(),
            CURSOR_INSPECT_NAME_CHARS + CURSOR_INSPECT_NAME_ELLIPSIS.len());
    }
}
