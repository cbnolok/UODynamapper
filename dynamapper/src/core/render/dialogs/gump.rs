use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionToggleGumpDialog};
use crate::core::render::scene::camera::UiCameraResource;
use crate::core::uo_files_loader::{ClassicHuesRes, GumpMapRes, TileMetaPackageRes};
use crate::ingame_sysmessage_logger;
use crate::{
    core::render::dialogs,
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass, EguiTextureHandle, EguiUserTextures};

#[derive(Resource)]
pub struct GumpDialogState {
    pub open: bool,
    pub id: String,
    open_gumps: Vec<OpenGump>,
    pub paperdoll_open: bool,
    pub paperdoll_female: bool,
    pub paperdoll_body_id: String,
    pub paperdoll_body_hue: String,
    paperdoll_equipment: Vec<PaperdollEquipmentRow>,
}

struct OpenGump {
    title: String,
    width: u16,
    height: u16,
    image: Handle<Image>,
    texture_id: egui::TextureId,
}

#[derive(Clone)]
struct PaperdollEquipmentRow {
    item_id: String,
    hue: String,
}

impl Default for PaperdollEquipmentRow {
    fn default() -> Self {
        Self {
            item_id: String::new(),
            hue: "0".to_string(),
        }
    }
}

impl Default for GumpDialogState {
    fn default() -> Self {
        Self {
            open: false,
            id: String::new(),
            open_gumps: Vec::new(),
            paperdoll_open: false,
            paperdoll_female: false,
            paperdoll_body_id: String::new(),
            paperdoll_body_hue: "0".to_string(),
            paperdoll_equipment: vec![PaperdollEquipmentRow::default(); 6],
        }
    }
}

pub struct GumpDialogPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(GumpDialogPlugin);

impl Plugin for GumpDialogPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<GumpDialogState>()
            .add_observer(sys_gump_toggle)
            .add_observer(sys_gump_close)
            .add_systems(
                EguiPrimaryContextPass,
                sys_render_gump_dialog.run_if(in_state(AppState::InGame)),
            );
    }
}

fn sys_gump_toggle(
    _trigger: On<ActionToggleGumpDialog>,
    mut state: ResMut<GumpDialogState>,
) {
    log_system_add_one_shot::<GumpDialogPlugin>("Observer", "ActionToggleGumpDialog", fname!());
    state.open = !state.open;
}

fn sys_gump_close(
    _trigger: On<ActionCloseActiveDialog>,
    mut state: ResMut<GumpDialogState>,
) {
    log_system_add_one_shot::<GumpDialogPlugin>("Observer", "ActionCloseActiveDialog", fname!());
    state.open = false;
}

pub fn sys_render_gump_dialog(
    mut state: ResMut<GumpDialogState>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    gump_map: Option<Res<GumpMapRes>>,
    tilemeta: Option<Res<TileMetaPackageRes>>,
    classic_hues: Option<Res<ClassicHuesRes>>,
    mut images: ResMut<Assets<Image>>,
    mut egui_user_textures: ResMut<EguiUserTextures>,
) {
    if !state.open && state.open_gumps.is_empty() {
        return;
    }

    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    if state.open {
        render_open_gump_dialog(
            ctx,
            &mut state,
            gump_map.as_deref(),
            tilemeta.as_deref(),
            classic_hues.as_deref(),
            &mut images,
            &mut egui_user_textures,
        );
    }

    let mut closed_images = Vec::new();
    for open_gump in &state.open_gumps {
        let mut open = true;
        let mut right_clicked = false;
        let image_size = egui::vec2(open_gump.width as f32, open_gump.height as f32);
        egui::Window::new(open_gump.title.as_str())
            .default_pos([
                120.0 + (state.open_gumps.len() as f32 * 12.0),
                120.0 + (state.open_gumps.len() as f32 * 12.0),
            ])
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let response = ui.add(egui::Image::new((open_gump.texture_id, image_size)));
                if response.secondary_clicked() {
                    right_clicked = true;
                }
            });

        if !open || right_clicked {
            closed_images.push(open_gump.image.id());
        }
    }

    state.open_gumps.retain(|open_gump| {
        let close = closed_images.contains(&open_gump.image.id());
        if close {
            images.remove(open_gump.image.id());
        }
        !close
    });

    for image_id in closed_images {
        egui_user_textures.remove_image(image_id);
    }
}

fn render_open_gump_dialog(
    ctx: &mut egui::Context,
    state: &mut GumpDialogState,
    gump_map: Option<&GumpMapRes>,
    tilemeta: Option<&TileMetaPackageRes>,
    classic_hues: Option<&ClassicHuesRes>,
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
) {
    let mut window_open = state.open;
    egui::Window::new("Open Gump")
        .default_pos([400.0, 240.0])
        .default_size([300.0, 260.0])
        .collapsible(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("ID:");
                ui.text_edit_singleline(&mut state.id);
            });

            let submit = ui.button("Open").clicked()
                || ui.input(|input| input.key_pressed(egui::Key::Enter));
            if submit {
                open_gump_by_id(state, gump_map, images, egui_user_textures);
            }

            ui.separator();
            ui.checkbox(&mut state.paperdoll_open, "Paperdoll");
            if state.paperdoll_open {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.paperdoll_female, false, "Male");
                    ui.radio_value(&mut state.paperdoll_female, true, "Female");
                });
                ui.horizontal(|ui| {
                    ui.label("Body:");
                    ui.text_edit_singleline(&mut state.paperdoll_body_id);
                    ui.label("Hue:");
                    ui.text_edit_singleline(&mut state.paperdoll_body_hue);
                });

                egui::Grid::new("paperdoll_equipment_grid")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Item");
                        ui.label("Hue");
                        ui.end_row();

                        for row in &mut state.paperdoll_equipment {
                            ui.text_edit_singleline(&mut row.item_id);
                            ui.text_edit_singleline(&mut row.hue);
                            ui.end_row();
                        }
                    });

                ui.horizontal(|ui| {
                    if ui.button("Add Row").clicked() {
                        state.paperdoll_equipment.push(PaperdollEquipmentRow::default());
                    }
                    if ui.button("Open Paperdoll").clicked() {
                        open_paperdoll(
                            state,
                            gump_map,
                            tilemeta,
                            classic_hues,
                            images,
                            egui_user_textures,
                        );
                    }
                });
            }
        });

    state.open = window_open;
}

fn open_gump_by_id(
    state: &mut GumpDialogState,
    gump_map: Option<&GumpMapRes>,
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
) {
    let Some(gump_map) = gump_map else {
        ingame_sysmessage_logger::error("Classic gump source is not loaded.".to_string());
        return;
    };

    let Ok(id) = state.id.trim().parse::<u32>() else {
        ingame_sysmessage_logger::error(format!("Invalid gump ID: {}", state.id));
        return;
    };

    let mut scratch = Vec::new();
    let (width, height, pixels) = match gump_map.0.decode_gump(id, &mut scratch) {
        Ok(gump) => gump,
        Err(error) => {
            ingame_sysmessage_logger::error(format!("Could not open gump {id}: {error}"));
            return;
        }
    };

    let mut image = crate::util_lib::image::image_from_rgba8(
        width as u32,
        height as u32,
        &pixels,
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    let image = images.add(image);
    let texture_id = egui_user_textures.add_image(EguiTextureHandle::Strong(image.clone()));
    state.open_gumps.push(OpenGump {
        title: format!("Gump {id}"),
        width,
        height,
        image,
        texture_id,
    });
}

struct PaperdollLayer {
    gump_id: u32,
    hue_id: u16,
    partial_hue: bool,
    sort_key: u16,
}

fn open_paperdoll(
    state: &mut GumpDialogState,
    gump_map: Option<&GumpMapRes>,
    tilemeta: Option<&TileMetaPackageRes>,
    classic_hues: Option<&ClassicHuesRes>,
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
) {
    let Some(gump_map) = gump_map else {
        ingame_sysmessage_logger::error("Classic gump source is not loaded.".to_string());
        return;
    };
    let Some(tilemeta) = tilemeta else {
        ingame_sysmessage_logger::error("Tile metadata is not loaded.".to_string());
        return;
    };

    let Ok(body_id) = parse_u32_field(&state.paperdoll_body_id) else {
        ingame_sysmessage_logger::error(format!(
            "Invalid paperdoll body gump ID: {}",
            state.paperdoll_body_id
        ));
        return;
    };
    let body_hue = parse_optional_hue(&state.paperdoll_body_hue);

    let mut layers = vec![PaperdollLayer {
        gump_id: body_id,
        hue_id: body_hue,
        partial_hue: false,
        sort_key: 0,
    }];

    let equipment_offset = if state.paperdoll_female { 60_000 } else { 50_000 };
    for row in &state.paperdoll_equipment {
        if row.item_id.trim().is_empty() {
            continue;
        }

        let Ok(item_id) = parse_u32_field(&row.item_id) else {
            ingame_sysmessage_logger::error(format!("Invalid equipment item ID: {}", row.item_id));
            return;
        };
        let Some(item) = tilemeta.0.item_tile(item_id) else {
            ingame_sysmessage_logger::error(format!("Unknown equipment item ID: {item_id}"));
            return;
        };
        if item.anim_id == 0 {
            ingame_sysmessage_logger::error(format!(
                "Equipment item {item_id} has no paperdoll anim_id."
            ));
            return;
        }

        layers.push(PaperdollLayer {
            gump_id: item.anim_id as u32 + equipment_offset,
            hue_id: parse_optional_hue(&row.hue),
            partial_hue: (item.flags & 0x40000) != 0,
            sort_key: u16::from(item.quality),
        });
    }

    layers[1..].sort_by_key(|layer| layer.sort_key);

    let (width, height, pixels) = match compose_paperdoll(gump_map, classic_hues, &layers) {
        Ok(composed) => composed,
        Err(error) => {
            ingame_sysmessage_logger::error(format!("Could not compose paperdoll: {error}"));
            return;
        }
    };

    let mut image = crate::util_lib::image::image_from_rgba8(width, height, &pixels);
    image.sampler = bevy::image::ImageSampler::nearest();
    let image = images.add(image);
    let texture_id = egui_user_textures.add_image(EguiTextureHandle::Strong(image.clone()));
    state.open_gumps.push(OpenGump {
        title: "Paperdoll".to_string(),
        width: width as u16,
        height: height as u16,
        image,
        texture_id,
    });
}

fn parse_u32_field(text: &str) -> Result<u32, std::num::ParseIntError> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse::<u32>()
    }
}

fn parse_optional_hue(text: &str) -> u16 {
    parse_u32_field(text).unwrap_or(0).min(u16::MAX as u32) as u16
}

fn compose_paperdoll(
    gump_map: &GumpMapRes,
    classic_hues: Option<&ClassicHuesRes>,
    layers: &[PaperdollLayer],
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    let mut decoded_layers = Vec::with_capacity(layers.len());
    let mut scratch = Vec::new();
    let mut width = 0u32;
    let mut height = 0u32;

    for layer in layers {
        let (layer_width, layer_height, mut pixels) =
            gump_map.0.decode_gump(layer.gump_id, &mut scratch)?;
        if layer.hue_id > 0 {
            apply_hue(&mut pixels, classic_hues, layer.hue_id, layer.partial_hue)?;
        }
        width = width.max(layer_width as u32);
        height = height.max(layer_height as u32);
        decoded_layers.push((layer_width as u32, layer_height as u32, pixels));
    }

    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    for (layer_width, layer_height, pixels) in decoded_layers {
        alpha_blend_top_left(&mut canvas, width, layer_width, layer_height, &pixels);
    }

    Ok((width, height, canvas))
}

fn apply_hue(
    pixels: &mut [u8],
    classic_hues: Option<&ClassicHuesRes>,
    hue_id: u16,
    partial_hue: bool,
) -> color_eyre::eyre::Result<()> {
    let hue = classic_hues
        .and_then(|hues| hues.0.get(hue_id.saturating_sub(1) as usize))
        .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} is not loaded."))?;

    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        if partial_hue && (pixel[0] != pixel[1] || pixel[0] != pixel[2]) {
            continue;
        }

        let intensity = (((pixel[0] as u16 >> 3)
            + (pixel[1] as u16 >> 3)
            + (pixel[2] as u16 >> 3))
            / 3)
            .min(31) as usize;
        let color = hue.color_table[intensity];
        pixel[0] = (((color >> 10) & 0x1F) as u8) << 3;
        pixel[1] = (((color >> 5) & 0x1F) as u8) << 3;
        pixel[2] = ((color & 0x1F) as u8) << 3;
    }

    Ok(())
}

fn alpha_blend_top_left(
    canvas: &mut [u8],
    canvas_width: u32,
    layer_width: u32,
    layer_height: u32,
    layer: &[u8],
) {
    for y in 0..layer_height as usize {
        for x in 0..layer_width as usize {
            let src_i = (y * layer_width as usize + x) * 4;
            let src_a = layer[src_i + 3] as u16;
            if src_a == 0 {
                continue;
            }

            let dst_i = (y * canvas_width as usize + x) * 4;
            if src_a == 255 {
                canvas[dst_i..dst_i + 4].copy_from_slice(&layer[src_i..src_i + 4]);
                continue;
            }

            let inv_a = 255 - src_a;
            canvas[dst_i] =
                ((layer[src_i] as u16 * src_a + canvas[dst_i] as u16 * inv_a) / 255) as u8;
            canvas[dst_i + 1] =
                ((layer[src_i + 1] as u16 * src_a + canvas[dst_i + 1] as u16 * inv_a) / 255) as u8;
            canvas[dst_i + 2] =
                ((layer[src_i + 2] as u16 * src_a + canvas[dst_i + 2] as u16 * inv_a) / 255) as u8;
            canvas[dst_i + 3] = (src_a + (canvas[dst_i + 3] as u16 * inv_a) / 255) as u8;
        }
    }
}
