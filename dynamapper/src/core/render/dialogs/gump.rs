use crate::core::controls::input_actions::{ActionCloseActiveDialog, ActionToggleGumpDialog};
use crate::core::render::scene::camera::UiCameraResource;
use crate::core::uo_files_loader::GumpMapRes;
use crate::ingame_sysmessage_logger;
use crate::{
    core::render::dialogs,
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass, EguiTextureHandle, EguiUserTextures};

#[derive(Resource, Default)]
pub struct GumpDialogState {
    pub open: bool,
    pub id: String,
    open_gumps: Vec<OpenGump>,
}

struct OpenGump {
    id: u32,
    width: u16,
    height: u16,
    image: Handle<Image>,
    texture_id: egui::TextureId,
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
            &mut images,
            &mut egui_user_textures,
        );
    }

    let mut closed_images = Vec::new();
    for open_gump in &state.open_gumps {
        let mut open = true;
        let mut right_clicked = false;
        let image_size = egui::vec2(open_gump.width as f32, open_gump.height as f32);
        egui::Window::new(format!("Gump {}", open_gump.id))
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
    images: &mut Assets<Image>,
    egui_user_textures: &mut EguiUserTextures,
) {
    let mut window_open = state.open;
    egui::Window::new("Open Gump")
        .default_pos([400.0, 240.0])
        .fixed_size([170.0, 90.0])
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
        id,
        width,
        height,
        image,
        texture_id,
    });
}
