//! The ground around a mobile, where the art shader finds the cell each pixel below its anchor lies on
//! (art/mobile_ground.wesl `mobile_ground_hit`, art/depth.wesl `mobile_fragment_view_z`): the land as drawn and the
//! Background floors and carpets, each a rise over the mobile's z up to `MOBILE_GROUND_REACH_Z`.
//!
//! A pixel's line of sight meets the ground where the ground is drawn: on flat ground at the mobile's z where it
//! reaches that z, on a carpet a z higher or on land rising in front as soon as it reaches them. Taken on a plane over
//! the base instead, the pixel lay a little nearer everywhere: no seam of the land or a rug in front over the legs, but
//! the foot of a post or a trunk in front under a hoof on flat ground. Read where the ground is, neither happens.
//!
//! The ground is read when the mobile's cell, z or map changes, or the map data, the caves drawn, the z-view's limit on
//! the land or the statics' art source does.

use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use crate::content::{MapPlanesRes, TileMetaPackageRes};
use crate::render::scene::SceneStateData;
use crate::render::statics::static_source::StaticArtSourceState;
use crate::settings::{ClientTextureSource, Settings};
use crate::world::caves::CavesRes;
use crate::world::coords::scale_bevy_units_to_uo_z;
use crate::world::multis::{MultiDefinitionsRes, PlacedMultisRes};
use crate::world::statics::StaticsStoreRes;
use crate::world::z_view::{visit_region_statics, PackageLand, ZViewLimit};
use uocf::enhanced::tileart::TaeFlag;
use super::draw::mobile_depth_cell;

/// The cells along a side of the ground read (art/mobile_ground.wesl `MOBILE_GROUND_DIM`): from
/// `MOBILE_GROUND_FIRST_CELL_OFFSET` cells behind the mobile's to 10 in front, as far as a frame's pixels 120 below its
/// anchor and 120 to either side reach the ground. Past them the ground lies flat at the mobile's z.
pub const MOBILE_GROUND_DIM: i32 = 16;
/// The first cell read from the mobile's, along x and y.
const MOBILE_GROUND_FIRST_CELL_OFFSET: i32 = -5;
/// How high over the mobile's z the ground is read (art/depth.wesl `MOBILE_GROUND_REACH_Z`): higher land is read as
/// this high, higher floors not at all; either covers the pixels whatever the cell they lie on.
pub const MOBILE_GROUND_REACH_Z: i32 = 2;
/// The bits of a cell, and the cells in a word of `MobileGround::cells`.
const GROUND_CELL_BITS: usize = 4;
const GROUND_CELLS_PER_WORD: usize = u32::BITS as usize / GROUND_CELL_BITS;
const GROUND_WORDS: usize = (MOBILE_GROUND_DIM * MOBILE_GROUND_DIM) as usize / GROUND_CELLS_PER_WORD;
/// The land code of a cell whose land is not drawn, and where a cell's floors sit in its code (`ground_cell_code`).
const GROUND_LAND_HIDDEN: u32 = 3;
const GROUND_FLOOR_SHIFT: usize = 2;
/// `MobileGround::flags`: the ground is read; the floors' art reaches past their cells.
const GROUND_PRESENT: u32 = 1;
const GROUND_FLOORS_OVERHANG: u32 = 2;
/// The statics that are ground: the floors, carpets and cave floors, lying flat (of height 0).
const FLOOR_FLAGS: u64 = TaeFlag::Background.bits() | TaeFlag::Surface.bits();
const NEVER_DRAWN_FLAGS: u64 = TaeFlag::NoDraw.bits();

/// The ground around a mobile, as the art sprite shader reads it (art/art_bindings.wesl `MobileGround`): the first of
/// its `MOBILE_GROUND_DIM` x `MOBILE_GROUND_DIM` cells, the z its rises count from, its flags, and the cells' codes
/// (`ground_cell_code`), 8 a word along x then y. All zero, it is not read: the shader takes flat ground at the base.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MobileGround {
    pub first_cell: [i32; 2],
    pub base_z: f32,
    pub flags: u32,
    pub cells: [u32; GROUND_WORDS],
}

/// A cell's code: the land's rise over the base at the cell's top corner in bits 0 and 1, clamped to 0 to
/// `MOBILE_GROUND_REACH_Z`, or `GROUND_LAND_HIDDEN` where the cell's land is not drawn; in bits 2 and 3 the highest
/// rise of its floors within reach, + 1, or 0 for none.
fn ground_cell_code(land_rise: Option<i32>, floor_rise: Option<i32>) -> u32 {
    let land = land_rise.map_or(GROUND_LAND_HIDDEN, |rise| rise.clamp(0, MOBILE_GROUND_REACH_Z) as u32);
    let floors = floor_rise.map_or(0, |rise| rise as u32 + 1);
    land | floors << GROUND_FLOOR_SHIFT
}

/// The ground around the mobile on the cell (`x`, `y`) at `z`: `land_z` the z of a cell's top corner as drawn, `None`
/// where its land is not; `floors` the cell and z of each floor around; `floors_overhang` whether the floors' art reaches
/// past their cells.
pub fn read_mobile_ground(x: i32, y: i32, z: i32, mut land_z: impl FnMut(i32, i32) -> Option<i32>,
    floors: impl IntoIterator<Item = (i32, i32, i32)>, floors_overhang: bool) -> MobileGround {
    let first = IVec2::new(x, y) + IVec2::splat(MOBILE_GROUND_FIRST_CELL_OFFSET);
    let dim = MOBILE_GROUND_DIM as usize;
    let mut floor_rises = vec![None; dim * dim];
    for (floor_x, floor_y, floor_z) in floors {
        let local = IVec2::new(floor_x, floor_y) - first;
        let rise = floor_z - z;
        if local.min_element() < 0 || local.max_element() >= MOBILE_GROUND_DIM || !(0..=MOBILE_GROUND_REACH_Z).contains(&rise) {
            continue;
        }
        let slot: &mut Option<i32> = &mut floor_rises[local.y as usize * dim + local.x as usize];
        *slot = Some(slot.map_or(rise, |highest| highest.max(rise)));
    }
    let mut ground = MobileGround { first_cell: first.to_array(), base_z: z as f32,
        flags: GROUND_PRESENT | if floors_overhang { GROUND_FLOORS_OVERHANG } else { 0 }, cells: [0; GROUND_WORDS] };
    for (index, floor_rise) in floor_rises.into_iter().enumerate() {
        let (local_x, local_y) = ((index % dim) as i32, (index / dim) as i32);
        let code = ground_cell_code(land_z(first.x + local_x, first.y + local_y).map(|land| land - z), floor_rise);
        ground.cells[index / GROUND_CELLS_PER_WORD] |= code << ((index % GROUND_CELLS_PER_WORD) * GROUND_CELL_BITS);
    }
    ground
}

/// The ground around a mobile, read when what it is read from changes (`sys_update_mobile_grounds`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MobileGroundCache {
    inputs: Option<MobileGroundInputs>,
    pub ground: MobileGround,
}

/// What a mobile's ground was read from: the map, the cell the mobile is ordered from and its z, the hidden multi parts
/// shown, the caves drawn (`CavesRes::revision`), the z-view's limit on the land and whether the floors' art reaches
/// past their cells.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MobileGroundInputs {
    map_id: u32,
    x: i32,
    y: i32,
    z: i32,
    show_hidden_multi_parts: bool,
    caves_revision: u64,
    land_max_z: f32,
    floors_overhang: bool,
}

/// Reads the ground around each mobile whose cell, z or map changed, or all when the map data, the caves, the z-view's
/// limit on the land or the statics' art source did. The floors' art reaches past their cells with the Enhanced art,
/// whose floors are camera quads larger than their cells (art/mobile_ground.wesl `MOBILE_GROUND_FLOOR_OVERHANG`).
#[allow(clippy::too_many_arguments)]
pub fn sys_update_mobile_grounds(
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    multis: (Option<Res<MultiDefinitionsRes>>, Res<PlacedMultisRes>),
    map_planes: Option<Res<MapPlanesRes>>,
    (caves, z_view, art_source): (Res<CavesRes>, Res<ZViewLimit>, Option<Res<StaticArtSourceState>>),
    mut mobiles_q: Query<(&Transform, &mut MobileGroundCache)>,
    mut land: Local<PackageLand>,
) {
    let Some(tilemeta) = tilemeta_res else { return };
    let map_id = scene_state.map_id;
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(Option::as_ref) else { return };
    let data_changed = statics_res.is_changed() || tilemeta.is_changed()
        || multis.0.as_ref().is_some_and(|definitions| definitions.is_changed()) || multis.1.is_changed();
    let show_hidden_multi_parts = settings.world_rendering.show_hidden_multi_parts;
    let floors_overhang = art_source.is_some_and(|state| state.active_source == Some(ClientTextureSource::Ec));
    let land_max_z = z_view.0.land_max_z();
    for (transform, mut cache) in &mut mobiles_q {
        let (x, y) = mobile_depth_cell(transform.translation.x, transform.translation.z);
        let inputs = MobileGroundInputs { map_id, x: x as i32, y: y as i32,
            z: scale_bevy_units_to_uo_z(transform.translation.y).round() as i32, show_hidden_multi_parts,
            caves_revision: caves.revision(), land_max_z, floors_overhang };
        if !data_changed && cache.inputs == Some(inputs) {
            continue;
        }
        let tilemeta = tilemeta.for_world_analyses();
        let mut floors = Vec::new();
        let half = MOBILE_GROUND_DIM / 2;
        let center = (inputs.x + MOBILE_GROUND_FIRST_CELL_OFFSET + half, inputs.y + MOBILE_GROUND_FIRST_CELL_OFFSET + half);
        visit_region_statics(&mut statics_store.lock(), multis.0.as_deref(), &multis.1, show_hidden_multi_parts,
            map_id, center, half, |x, y, graphic, z| {
                let Some(meta) = tilemeta.item_tile(u32::from(graphic)) else { return };
                if meta.height == 0 && meta.flags & FLOOR_FLAGS == FLOOR_FLAGS && meta.flags & NEVER_DRAWN_FLAGS == 0 {
                    floors.push((x, y, i32::from(z)));
                }
            }, |_| {});
        let active_caves = caves.active();
        let land_z = |cell_x: i32, cell_y: i32| {
            let cave_floor = active_caves.zip(u32::try_from(cell_x).ok().zip(u32::try_from(cell_y).ok()))
                .and_then(|(caves, (cell_x, cell_y))| caves.floor_z(cell_x, cell_y)).map(i32::from);
            cave_floor.or_else(|| land.land_z(map_planes.as_deref(), map_id, cell_x, cell_y))
                .filter(|&land_z| land_z as f32 <= land_max_z)
        };
        cache.ground = read_mobile_ground(inputs.x, inputs.y, inputs.z, land_z, floors, floors_overhang);
        cache.inputs = Some(inputs);
    }
}

/// art/mobile_ground.wesl `mobile_ground_hit` in the isometric camera, whose line of sight runs along (1, 1, 1) toward
/// it, for the tests of the mobiles' depth (draw.rs).
#[cfg(test)]
pub(super) mod mirror {
    use super::*;
    use crate::util_lib::wesl_source::{wesl_const, wesl_f32_const};
    use crate::world::coords::scale_uo_z_to_bevy_units;

    pub const DEPTH_WESL: &str = include_str!("../../../assets/shaders/world/art/depth.wesl");
    pub const GROUND_WESL: &str = include_str!("../../../assets/shaders/world/art/mobile_ground.wesl");
    const ISOMETRIC_SIGHT: Vec3 = Vec3::ONE;
    /// The half diagonal of an Enhanced camera quad and of its cell's diamond (ec_item_quad.wesl), and the overhang of
    /// the quad they give.
    const EC_CAMERA_QUAD_HALF_DIAGONAL: f32 = 0.7671049;
    const CELL_HALF_DIAGONAL: f32 = 0.707105;
    pub const FLOOR_OVERHANG: f32 = (EC_CAMERA_QUAD_HALF_DIAGONAL / CELL_HALF_DIAGONAL - 1.0) * 0.5;
    const MAX_CELLS: usize = 8;
    const FIELD_MASK: u32 = 3;
    const CELL_MASK: u32 = 0xF;

    pub fn reach_z() -> f32 {
        wesl_f32_const(DEPTH_WESL, "MOBILE_GROUND_REACH_Z")
    }

    pub fn tie_z() -> f32 {
        wesl_f32_const(DEPTH_WESL, "MOBILE_GROUND_TIE_Z")
    }

    /// The constants the shader shares with this file.
    pub fn check_shader_constants() {
        assert_eq!(reach_z(), MOBILE_GROUND_REACH_Z as f32);
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_DIM"), MOBILE_GROUND_DIM.to_string());
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_FLOOR_OVERHANG"),
            format!("({EC_CAMERA_QUAD_HALF_DIAGONAL} / {CELL_HALF_DIAGONAL} - 1.0) * 0.5"));
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_LAND_HIDDEN"), format!("{GROUND_LAND_HIDDEN}u"));
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_FLOOR_SHIFT"), format!("{GROUND_FLOOR_SHIFT}u"));
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_PRESENT"), format!("{GROUND_PRESENT}u"));
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_FLOORS_OVERHANG"), format!("{GROUND_FLOORS_OVERHANG}u"));
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_CELL_BITS"), format!("{GROUND_CELL_BITS}u"));
        assert_eq!(wesl_const(GROUND_WESL, "MOBILE_GROUND_CELLS_PER_WORD"), format!("{GROUND_CELLS_PER_WORD}u"));
    }

    fn plane_hit(point: Vec3, plane_y: f32) -> Vec2 {
        let hit = point + ISOMETRIC_SIGHT * (plane_y - point.y);
        Vec2::new(hit.x, hit.z)
    }

    pub fn code(ground: &MobileGround, cell: IVec2) -> u32 {
        let local = cell - IVec2::from_array(ground.first_cell);
        if local.min_element() < 0 || local.max_element() >= MOBILE_GROUND_DIM {
            return 0;
        }
        let index = (local.y * MOBILE_GROUND_DIM + local.x) as usize;
        ground.cells[index / GROUND_CELLS_PER_WORD] >> ((index % GROUND_CELLS_PER_WORD) * GROUND_CELL_BITS) & CELL_MASK
    }

    fn corner_rise(ground: &MobileGround, cell: IVec2) -> f32 {
        ((code(ground, cell) & FIELD_MASK) as f32).min(reach_z())
    }

    fn land_rise(ground: &MobileGround, cell: IVec2, point: Vec2) -> f32 {
        let [top, right, left, bottom] = [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE]
            .map(|corner| corner_rise(ground, cell + corner));
        let inside = (point - cell.as_vec2()).clamp(Vec2::ZERO, Vec2::ONE);
        let near = top + (right - top) * inside.x;
        let far = left + (bottom - left) * inside.x;
        near + (far - near) * inside.y
    }

    fn leaves(start: Vec2, span: Vec2, corner: Vec2, grown: f32) -> Vec2 {
        let along = |axis: usize| {
            if span[axis] == 0.0 {
                return 1.0;
            }
            let edge = if span[axis] > 0.0 { corner[axis] + 1.0 + grown } else { corner[axis] - grown };
            (edge - start[axis]) / span[axis]
        };
        Vec2::new(along(0), along(1))
    }

    fn line_rise(t: f32) -> f32 {
        reach_z() + (tie_z() - reach_z()) * t.clamp(0.0, 1.0)
    }

    /// The cell where the line of sight through `point` meets `ground`.
    pub fn ground_hit_cell(point: Vec3, ground: &MobileGround) -> (f32, f32) {
        let base_y = scale_uo_z_to_bevy_units(ground.base_z);
        let far = plane_hit(point, base_y + scale_uo_z_to_bevy_units(tie_z()));
        let start = plane_hit(point, base_y + scale_uo_z_to_bevy_units(reach_z()));
        let span = far - start;
        let overhang = if ground.flags & GROUND_FLOORS_OVERHANG != 0 { FLOOR_OVERHANG } else { 0.0 };
        let mut cell = start.floor().as_ivec2();
        for _ in 0..MAX_CELLS {
            let corner = cell.as_vec2();
            let code = code(ground, cell);
            let leaving = leaves(start, span, corner, 0.0);
            let t = leaving.min_element().clamp(0.0, 1.0);
            let (land, floors) = (code & FIELD_MASK, code >> GROUND_FLOOR_SHIFT & FIELD_MASK);
            let mut meets = land != GROUND_LAND_HIDDEN
                && line_rise(t) <= land_rise(ground, cell, start + span * t) + tie_z();
            if floors != 0 {
                meets |= line_rise(leaves(start, span, corner, overhang).min_element())
                    <= (floors - 1) as f32 + tie_z();
            }
            if meets {
                return (corner.x, corner.y);
            }
            if t >= 1.0 {
                break;
            }
            let step = span.signum().as_ivec2();
            cell += IVec2::new(if leaving.x <= t { step.x } else { 0 }, if leaving.y <= t { step.y } else { 0 });
        }
        (far.x.floor(), far.y.floor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const X: i32 = 1473;
    const Y: i32 = 1722;
    const Z: i32 = 10;

    #[test]
    fn the_shader_reads_the_ground_as_this_file_packs_it() {
        mirror::check_shader_constants();
        // first_cell, base_z and flags (16 bytes), and the cells' 32 words.
        const WGSL_MOBILE_GROUND_SIZE: usize = 144;
        assert_eq!(std::mem::size_of::<MobileGround>(), WGSL_MOBILE_GROUND_SIZE);
        assert_eq!(GROUND_WORDS * u32::BITS as usize,
            (MOBILE_GROUND_DIM * MOBILE_GROUND_DIM) as usize * GROUND_CELL_BITS);
    }

    /// The land rises over the mobile's z as far as the ground is read and no lower than it; land not drawn is read as
    /// none. The highest floor within reach is the cell's, a floor over it or under the mobile none.
    #[test]
    fn the_ground_holds_the_land_and_the_floors_within_reach() {
        const HIDDEN_X: i32 = X + 3;
        let land_z = |x: i32, y: i32| (x != HIDDEN_X).then_some(Z + (x - X) + (y - Y));
        let floors = [(X + 1, Y, Z + 1), (X + 1, Y, Z), (X + 2, Y, Z + 3), (X, Y + 1, Z - 1), (X, Y + 2, Z + 2)];
        let ground = read_mobile_ground(X, Y, Z, land_z, floors, false);
        assert_eq!(ground.first_cell, [X + MOBILE_GROUND_FIRST_CELL_OFFSET, Y + MOBILE_GROUND_FIRST_CELL_OFFSET]);
        assert_eq!((ground.base_z, ground.flags), (Z as f32, GROUND_PRESENT));
        let floor = |rise: u32| (rise + 1) << GROUND_FLOOR_SHIFT;
        let code = |x, y| mirror::code(&ground, IVec2::new(x, y));
        assert_eq!(code(X, Y), 0, "flat land at the mobile's z");
        assert_eq!(code(X - 1, Y), 0, "land below it");
        assert_eq!(code(X + 1, Y), 1 | floor(1), "a carpet over the boards");
        assert_eq!(code(X + 2, Y), 2, "land past reach, a floor over it");
        assert_eq!(code(HIDDEN_X, Y), GROUND_LAND_HIDDEN);
        assert_eq!(code(X, Y + 1), 1, "a floor under the mobile");
        assert_eq!(code(X, Y + 2), 2 | floor(2));
        assert!(read_mobile_ground(X, Y, Z, land_z, [], true).flags & GROUND_FLOORS_OVERHANG != 0);
    }

    /// art/mobile_ground.wesl passes naga's validation, as the GPU's shader module creation runs it: no art shader is
    /// composed in a test. Its imports are stood in for: the struct and constants as written, `sight_plane_hit` by a
    /// function of the same signature, and a fragment entry point reads a ground from a storage buffer.
    #[test]
    fn the_ground_shader_validates() {
        use wgpu::naga;
        const BINDINGS_WESL: &str = include_str!("../../../assets/shaders/world/art/art_bindings.wesl");
        const STANDINS: &str = "
const HEIGHT_SCALE: f32 = 1.0;
const MOBILE_GROUND_REACH_Z: f32 = 2.0;
const MOBILE_GROUND_TIE_Z: f32 = 0.0625;
fn sight_plane_hit(point: vec3<f32>, plane_y: f32) -> vec4<f32> { return vec4<f32>(point.x, plane_y, point.z, 1.0); }
@group(0) @binding(0) var<storage, read> grounds: array<MobileGround>;
@fragment fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return mobile_ground_hit(position.xyz, 0.0, grounds[u32(position.x)]);
}
";
        let structure = BINDINGS_WESL.split("struct MobileGround {").nth(1).and_then(|rest| rest.split_once('}'))
            .map(|(fields, _)| format!("struct MobileGround {{{fields}}}\n")).expect("MobileGround in art_bindings.wesl");
        let ground: String = mirror::GROUND_WESL.lines().filter(|line| !line.starts_with("import "))
            .map(|line| format!("{line}\n")).collect();
        let wgsl = format!("{structure}{STANDINS}{ground}");
        let module = naga::front::wgsl::parse_str(&wgsl).unwrap_or_else(|error| panic!("{}", error.emit_to_string(&wgsl)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|error| panic!("{}", error.emit_to_string(&wgsl)));
    }
}
