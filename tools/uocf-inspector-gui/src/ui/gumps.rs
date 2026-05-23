use crate::app::{GumpSource, PaperdollEquipmentInput, UopInspectorApp};
use eframe::egui;
use std::sync::Arc;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::uop_container::hash::hash_file_name_single;

#[derive(Clone, Copy)]
struct PaperdollProfile {
    id: &'static str,
    label: &'static str,
    equipment_offset: u32,
    canvas_width: u32,
    canvas_height: u32,
    body_x: i32,
    body_y: i32,
    equipment_x: i32,
    equipment_y: i32,
}

const PAPERDOLL_PROFILES: &[PaperdollProfile] = &[
    PaperdollProfile::new("human_male", "Human Male", 50_000, 260, 300),
    PaperdollProfile::new("human_female", "Human Female", 60_000, 260, 300),
    PaperdollProfile::new("elf_male", "Elf Male", 50_000, 260, 300),
    PaperdollProfile::new("elf_female", "Elf Female", 60_000, 260, 300),
    PaperdollProfile::new("gargoyle_male", "Gargoyle Male", 50_000, 300, 340),
    PaperdollProfile::new("gargoyle_female", "Gargoyle Female", 60_000, 300, 340),
];

impl PaperdollProfile {
    const fn new(
        id: &'static str,
        label: &'static str,
        equipment_offset: u32,
        canvas_width: u32,
        canvas_height: u32,
    ) -> Self {
        Self {
            id,
            label,
            equipment_offset,
            canvas_width,
            canvas_height,
            body_x: 0,
            body_y: 0,
            equipment_x: 0,
            equipment_y: 0,
        }
    }
}

struct PaperdollLayer {
    gump_id: u32,
    hue_id: u16,
    partial_hue: bool,
    sort_key: u16,
    x: i32,
    y: i32,
}

pub fn ui_gumps(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.selected_gump_source, GumpSource::Classic, "CC");
            ui.selectable_value(&mut app.selected_gump_source, GumpSource::Enhanced, "EC");
        });
        ui.separator();

        ui.heading("Gump");
        ui.horizontal(|ui| {
            ui.label("ID:");
            ui.text_edit_singleline(&mut app.selected_gump_id);
        });

        match parse_u32_field(&app.selected_gump_id) {
            Ok(gump_id) => {
                if let Some(handle) = app.get_gump_texture(ctx, app.selected_gump_source, gump_id) {
                    ui.label(format!("{}x{}", handle.size()[0], handle.size()[1]));
                    egui::ScrollArea::both()
                        .id_salt("gump_preview_scroll")
                        .max_height(320.0)
                        .show(ui, |ui| {
                            ui.image(&handle);
                        });
                } else if !app.selected_gump_id.trim().is_empty() {
                    ui.label("Gump not found in the selected source.");
                }
            }
            Err(_) if !app.selected_gump_id.trim().is_empty() => {
                ui.label("Invalid gump ID.");
            }
            Err(_) => {}
        }

        ui.separator();
        ui.heading("Paperdoll");
        egui::ComboBox::from_label("Profile")
            .selected_text(profile_by_id(&app.selected_paperdoll_profile).label)
            .show_ui(ui, |ui| {
                for profile in PAPERDOLL_PROFILES {
                    ui.selectable_value(
                        &mut app.selected_paperdoll_profile,
                        profile.id.to_string(),
                        profile.label,
                    );
                }
            });

        ui.horizontal(|ui| {
            ui.label("Body:");
            ui.text_edit_singleline(&mut app.paperdoll_body_id);
            ui.label("Hue:");
            ui.text_edit_singleline(&mut app.paperdoll_body_hue);
        });

        egui::Grid::new("uocf_inspector_paperdoll_equipment")
            .num_columns(3)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Item ID");
                ui.label("Hue");
                ui.end_row();

                for row in &mut app.paperdoll_equipment {
                    ui.text_edit_singleline(&mut row.item_id);
                    ui.text_edit_singleline(&mut row.hue);
                    ui.end_row();
                }
            });

        ui.horizontal(|ui| {
            if ui.button("Add Row").clicked() {
                app.paperdoll_equipment.push(PaperdollEquipmentInput::default());
            }
            if ui.button("Render").clicked() {
                match render_paperdoll(app, ctx) {
                    Ok(handle) => {
                        app.paperdoll_preview = Some(handle);
                        app.status_message = "Rendered paperdoll preview.".to_string();
                    }
                    Err(error) => {
                        app.paperdoll_preview = None;
                        app.status_message = format!("Could not render paperdoll: {error}");
                    }
                }
            }
        });

        if let Some(handle) = &app.paperdoll_preview {
            egui::ScrollArea::both()
                .id_salt("paperdoll_preview_scroll")
                .show(ui, |ui| {
                    ui.image(handle);
                });
        }
    });
}

fn profile_by_id(id: &str) -> PaperdollProfile {
    PAPERDOLL_PROFILES
        .iter()
        .copied()
        .find(|profile| profile.id == id)
        .unwrap_or(PAPERDOLL_PROFILES[0])
}

fn render_paperdoll(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
) -> color_eyre::eyre::Result<egui::TextureHandle> {
    let body_id = parse_u32_field(&app.paperdoll_body_id)?;
    let profile = profile_by_id(&app.selected_paperdoll_profile);
    let mut layers = vec![PaperdollLayer {
        gump_id: body_id,
        hue_id: parse_optional_hue(&app.paperdoll_body_hue),
        partial_hue: false,
        sort_key: 0,
        x: profile.body_x,
        y: profile.body_y,
    }];

    let tiledata = app.cc_tiledata.as_ref().map(Arc::clone);
    for row in &app.paperdoll_equipment {
        if row.item_id.trim().is_empty() {
            continue;
        }

        let item_id = parse_u32_field(&row.item_id)?;
        let item = tiledata
            .as_ref()
            .and_then(|tiledata| tiledata.item_tiles().get(item_id as usize))
            .ok_or_else(|| color_eyre::eyre::eyre!("Unknown equipment item ID {item_id}"))?;
        if item.anim_id == 0 {
            color_eyre::eyre::bail!("Equipment item {item_id} has no paperdoll anim_id.");
        }

        layers.push(PaperdollLayer {
            gump_id: item.anim_id as u32 + profile.equipment_offset,
            hue_id: parse_optional_hue(&row.hue),
            partial_hue: item.flags.partialhue(),
            sort_key: item.quality as u16,
            x: profile.equipment_x,
            y: profile.equipment_y,
        });
    }

    layers[1..].sort_by_key(|layer| layer.sort_key);
    let (width, height, pixels) = compose_paperdoll(app, profile, &layers)?;
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &pixels,
    );
    Ok(ctx.load_texture(
        format!("paperdoll_{:?}_{}", app.selected_gump_source, app.selected_paperdoll_profile),
        image,
        Default::default(),
    ))
}

fn compose_paperdoll(
    app: &UopInspectorApp,
    profile: PaperdollProfile,
    layers: &[PaperdollLayer],
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    let mut decoded_layers = Vec::with_capacity(layers.len());
    let mut min_x = 0i32;
    let mut min_y = 0i32;
    let mut max_x = profile.canvas_width as i32;
    let mut max_y = profile.canvas_height as i32;

    for layer in layers {
        let (layer_width, layer_height, mut pixels) =
            decode_gump_rgba(app, app.selected_gump_source, layer.gump_id)?;
        if layer.hue_id > 0 {
            apply_hue(app, &mut pixels, layer.hue_id, layer.partial_hue)?;
        }

        min_x = min_x.min(layer.x);
        min_y = min_y.min(layer.y);
        max_x = max_x.max(layer.x + layer_width as i32);
        max_y = max_y.max(layer.y + layer_height as i32);
        decoded_layers.push((layer.x, layer.y, layer_width, layer_height, pixels));
    }

    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    for (x, y, layer_width, layer_height, pixels) in decoded_layers {
        alpha_blend_at(
            &mut canvas,
            width,
            x - min_x,
            y - min_y,
            layer_width,
            layer_height,
            &pixels,
        );
    }

    Ok((width, height, canvas))
}

fn decode_gump_rgba(
    app: &UopInspectorApp,
    source: GumpSource,
    gump_id: u32,
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    match source {
        GumpSource::Classic => {
            let gumps = app
                .cc_gumps
                .as_ref()
                .ok_or_else(|| color_eyre::eyre::eyre!("CC gump source is not loaded."))?;
            let mut scratch = Vec::new();
            let (width, height, pixels) = gumps.decode_gump(gump_id, &mut scratch)?;
            Ok((width as u32, height as u32, pixels))
        }
        GumpSource::Enhanced => decode_ec_gump_rgba(app, gump_id),
    }
}

fn decode_ec_gump_rgba(
    app: &UopInspectorApp,
    gump_id: u32,
) -> color_eyre::eyre::Result<(u32, u32, Vec<u8>)> {
    for loaded in &app.uop_cache.loaded_uops {
        let is_interface = loaded
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("interface.uop"))
            .unwrap_or(false);
        if !is_interface {
            continue;
        }

        let candidates = [
            (
                format!("data/interface/default/textures/gumpart/{:08}.tga", gump_id),
                ECImageFormat::TGA,
            ),
            (
                format!("data/interface/default/textures/gumpart/{:08}.dds", gump_id),
                ECImageFormat::DDS,
            ),
        ];
        for (path, format) in candidates {
            let hash = hash_file_name_single(&path);
            if let Some(file) = loaded.package.get_file_by_hash(hash) {
                let data: Arc<[u8]> = file.unpack()?.into();
                let tex_file = TextureFile {
                    metadata: RawTextureItem::absent(),
                    is_ec: true,
                    format,
                    props: None,
                    raw_data: data,
                };
                let image = tex_file.decode_to_rgba()?;
                let rgba = image.to_rgba8();
                return Ok((rgba.width(), rgba.height(), rgba.into_raw()));
            }
        }
    }

    color_eyre::eyre::bail!("EC gump {gump_id} was not found in interface.uop.");
}

fn apply_hue(
    app: &UopInspectorApp,
    pixels: &mut [u8],
    hue_id: u16,
    partial_hue: bool,
) -> color_eyre::eyre::Result<()> {
    let hue = app
        .client_data
        .as_ref()
        .and_then(|client| client.hues.as_ref())
        .and_then(|hues| hues.get(hue_id.saturating_sub(1) as usize))
        .ok_or_else(|| color_eyre::eyre::eyre!("Hue {hue_id} is not loaded."))?;

    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }

        let color = ((pixel[3] as u32) << 24)
            | ((pixel[0] as u32) << 16)
            | ((pixel[1] as u32) << 8)
            | pixel[2] as u32;
        let hued = hue.apply_to_color32(color, partial_hue);
        pixel[0] = ((hued >> 16) & 0xFF) as u8;
        pixel[1] = ((hued >> 8) & 0xFF) as u8;
        pixel[2] = (hued & 0xFF) as u8;
        pixel[3] = ((hued >> 24) & 0xFF) as u8;
    }

    Ok(())
}

fn alpha_blend_at(
    canvas: &mut [u8],
    canvas_width: u32,
    dst_x: i32,
    dst_y: i32,
    layer_width: u32,
    layer_height: u32,
    layer: &[u8],
) {
    for y in 0..layer_height as usize {
        for x in 0..layer_width as usize {
            let canvas_x = x as i32 + dst_x;
            let canvas_y = y as i32 + dst_y;
            if canvas_x < 0 || canvas_y < 0 {
                continue;
            }

            let src_i = (y * layer_width as usize + x) * 4;
            let src_a = layer[src_i + 3] as u16;
            if src_a == 0 {
                continue;
            }

            let dst_i = (canvas_y as usize * canvas_width as usize + canvas_x as usize) * 4;
            if dst_i + 4 > canvas.len() {
                continue;
            }

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
