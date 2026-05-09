use crate::logic::{ClientData, Dictionary, UopCache};
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;
use uocf::classic::art::ArtMap;
pub use uocf::classic::art::ArtSource;
use uocf::classic::tiledata::TileData;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::uop::package::{LoadMode, UopPackage};

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum ViewMode {
    UopExplorer,
    CcArt,
    CcTileData,
    Animations,
    Multis,
    Hues,
}

pub struct UopInspectorApp {
    pub dictionary: Dictionary,
    pub uo_string_dictionary: Option<Arc<UoStringDictionary>>,
    pub uop_cache: UopCache,
    pub client_data: Option<ClientData>,

    pub selected_uop_idx: Option<usize>,
    pub selected_file_hash: Option<u64>,
    pub selected_cc_art_id: Option<u32>,
    // pub selected_cc_tile_id: Option<u32>,
    pub selected_legacy_source: ArtSource,

    pub search_query: String,
    pub find_hash_query: String,
    pub status_message: String,
    pub view_mode: ViewMode,

    pub texture_previews: HashMap<u64, egui::TextureHandle>,
    pub ec_texture_previews: HashMap<u32, egui::TextureHandle>,

    // Animations
    pub selected_anim_id: u32,
    pub selected_anim_file_idx: u8,
    pub current_frame_idx: usize,
    pub is_playing: bool,
    pub last_frame_time: f64,
    pub playback_speed: f32,
    pub loop_animation: bool,

    pub selected_anim_sequence: Option<uocf::animation_sequence::AnimationSequence>,
    pub selected_action_id: u16,
    pub selected_direction: u8,

    pub selected_multi_id: u32,
    pub selected_hue_id: u16,
}

impl UopInspectorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        color_eyre::install().ok();
        Self {
            dictionary: Dictionary::new(),
            uo_string_dictionary: None,
            uop_cache: UopCache::new(),
            client_data: None,
            selected_uop_idx: None,
            selected_file_hash: None,
            selected_cc_art_id: None,
            // selected_cc_tile_id: None,
            selected_legacy_source: ArtSource::Any,
            search_query: String::new(),
            find_hash_query: String::new(),
            status_message: "Welcome to UOP Inspector".to_string(),
            view_mode: ViewMode::UopExplorer,
            texture_previews: HashMap::new(),
            ec_texture_previews: HashMap::new(),

            selected_anim_id: 0,
            selected_anim_file_idx: 0,
            current_frame_idx: 0,
            is_playing: false,
            last_frame_time: 0.0,
            playback_speed: 1.0,
            loop_animation: true,
            selected_anim_sequence: None,
            selected_action_id: 0,
            selected_direction: 0,
            selected_multi_id: 0,
            selected_hue_id: 0,
        }
    }

    pub fn open_uop(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("UOP Packages", &["uop"])
            .pick_file()
        {
            match UopPackage::load_with_mode(&path, LoadMode::Eager) {
                Ok(package) => {
                    self.uop_cache.add(path.clone(), package);
                    self.selected_uop_idx = Some(self.uop_cache.loaded_uops.len() - 1);
                    self.status_message = format!("Loaded {}", path.display());
                    self.view_mode = ViewMode::UopExplorer;
                }
                Err(e) => {
                    self.status_message = format!("Failed to load UOP: {}", e);
                }
            }
        }
    }

    pub fn open_client_dir(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Select UO Client Directory (containing .mul files)")
            .pick_folder()
        {
            let art_res = ArtMap::load(&path);
            let td_res = TileData::load(path.join("tiledata.mul"));

            match (art_res, td_res) {
                (Ok(art), Ok(td)) => {
                    let multis = uocf::classic::multi::MultiMap::load(&path)
                        .ok()
                        .map(Arc::new);
                    let hues = uocf::classic::hues::load_hues(&path.join("hues.mul"))
                        .ok()
                        .map(Arc::new);

                    // Try to load AnimationDefinition.uop if it exists in the UOP folder
                    let mut anim_defs = None;
                    let uop_dir = path.join("uop");
                    if uop_dir.exists() {
                        let anim_def_path = uop_dir.join("AnimationDefinition.uop");
                        if anim_def_path.exists() {
                            if let Ok(package) = UopPackage::load(&anim_def_path) {
                                let hash = uocf::uop::hash::hash_file_name_single(
                                    "data/animationdefinition/animationdefinition.bin",
                                );
                                if let Some(file) = package.get_file_by_hash(hash) {
                                    if let Ok(data) = file.unpack() {
                                        if let Ok(defs) =
                                            uocf::classic::anim::AnimationDefinition::parse(&data)
                                        {
                                            anim_defs = Some(Arc::new(defs));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    self.client_data = Some(ClientData {
                        path,
                        art: Arc::new(art),
                        tiledata: Arc::new(td),
                        multis,
                        _ec_multis: None,
                        hues,
                        anim_defs,
                    });
                    self.status_message =
                        "Loaded CC Art, TileData, Multis, Hues and AnimDefs".to_string();
                }
                (Err(e), _) | (_, Err(e)) => {
                    self.status_message = format!("Failed to load client data: {}", e);
                }
            }
        }
    }

    pub fn open_legacy_texture_uop(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open LegacyTexture.uop")
            .add_filter("UOP Packages", &["uop"])
            .pick_file()
        {
            match UopPackage::load(&path) {
                Ok(package) => {
                    if let Some(client) = &mut self.client_data {
                        let mut art = (*client.art).clone();
                        art = art.with_uop(package);
                        client.art = Arc::new(art);
                        self.status_message =
                            format!("Attached LegacyTexture.uop: {}", path.display());
                    } else {
                        let art = ArtMap::load_standalone_uop(package);
                        let parent = path
                            .parent()
                            .unwrap_or(std::path::Path::new("."))
                            .to_path_buf();
                        self.client_data = Some(ClientData {
                            path: parent,
                            art: Arc::new(art),
                            tiledata: Arc::new(TileData::new_empty()),
                            multis: None,
                            _ec_multis: None,
                            hues: None,
                            anim_defs: None,
                        });
                        self.status_message =
                            format!("Loaded standalone Legacy Art UOP: {}", path.display());
                    }
                }
                Err(e) => {
                    self.status_message = format!("Failed to load LegacyTexture.uop: {}", e);
                }
            }
        }
    }

    pub fn open_dictionary(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Dictionaries", &["dic", "txt"])
            .pick_file()
        {
            match self.dictionary.load(&path) {
                Ok(_) => {
                    self.status_message =
                        format!("Loaded dictionary with {} entries", self.dictionary.count());
                }
                Err(e) => {
                    self.status_message = format!("Failed to load dictionary: {}", e);
                }
            }
        }
    }

    pub fn open_bin_dictionary(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Binary Dictionaries", &["bin"])
            .pick_file()
        {
            match self.dictionary.load_bin(&path) {
                Ok(_) => {
                    self.status_message = format!(
                        "Loaded binary dictionary with {} entries",
                        self.dictionary.count()
                    );
                }
                Err(e) => {
                    self.status_message = format!("Failed to load binary dictionary: {}", e);
                }
            }
        }
    }

    pub fn open_uo_string_dictionary(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("UO String Dictionary", &["uop"])
            .pick_file()
        {
            match UoStringDictionary::load(&path) {
                Ok(dict) => {
                    self.uo_string_dictionary = Some(Arc::new(dict));
                    self.status_message = "Loaded UO String Dictionary".to_string();
                }
                Err(e) => {
                    self.status_message = format!("Failed to load UO String Dictionary: {}", e);
                }
            }
        }
    }

    pub fn get_uop_texture(
        &mut self,
        ctx: &egui::Context,
        hash: u64,
        data: &[u8],
        name: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.texture_previews.get(&hash) {
            return Some(handle.clone());
        }

        let format = if name.to_lowercase().ends_with(".dds") {
            ECImageFormat::DDS
        } else if name.to_lowercase().ends_with(".tga") {
            ECImageFormat::TGA
        } else {
            ECImageFormat::Unknown
        };

        let tex_file = TextureFile {
            metadata: RawTextureItem::absent(),
            is_ec: true,
            format,
            props: None,
            raw_data: data.into(),
        };

        match tex_file.decode_to_rgba() {
            Ok(img) => {
                let size = [img.width() as usize, img.height() as usize];
                let pixels = img.to_rgba8();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw());
                let handle = ctx.load_texture(name, color_image, Default::default());
                self.texture_previews.insert(hash, handle.clone());
                Some(handle)
            }
            Err(_) => None,
        }
    }

    pub fn save_entry(&mut self, hash: u64, name: &str) {
        if let Some(uop_idx) = self.selected_uop_idx {
            let loaded = &self.uop_cache.loaded_uops[uop_idx];
            if let Some(file) = loaded.package.get_file_by_hash(hash) {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(name)
                    .set_title("Extract Entry")
                    .save_file()
                {
                    match file.unpack() {
                        Ok(data) => {
                            if let Err(e) = std::fs::write(&path, data) {
                                self.status_message = format!("Failed to save: {}", e);
                            } else {
                                self.status_message = format!("Extracted to: {}", path.display());
                            }
                        }
                        Err(e) => {
                            self.status_message = format!("Failed to unpack: {}", e);
                        }
                    }
                }
            }
        }
    }

    pub fn get_ec_texture_by_id(
        &mut self,
        ctx: &egui::Context,
        texture_id: u32,
    ) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.ec_texture_previews.get(&texture_id) {
            return Some(handle.clone());
        }

        let loaded_uops = self.uop_cache.loaded_uops.clone();
        for loaded in &loaded_uops {
            let candidates = [
                format!("build/worldart/{:08}.dds", texture_id),
                format!("build/worldart/{:08}.tga", texture_id),
                format!("build/tileartlegacy/{:08}.dds", texture_id),
                format!("build/tileartlegacy/{:08}.tga", texture_id),
                format!("build/tileartenhanced/{:08}.dds", texture_id),
                format!("build/tileartenhanced/{:08}.tga", texture_id),
            ];

            for path in candidates {
                let hash = uocf::uop::hash::hash_file_name_single(&path);
                if let Some(file) = loaded.package.get_file_by_hash(hash) {
                    if let Ok(data) = file.unpack() {
                        if let Some(handle) = self.get_uop_texture(ctx, hash, &data, &path) {
                            self.ec_texture_previews.insert(texture_id, handle.clone());
                            return Some(handle);
                        }
                    }
                }
            }
        }
        None
    }

    fn get_cc_art_texture_from_source(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
        source: ArtSource,
    ) -> Option<egui::TextureHandle> {
        let hue_id = self.selected_hue_id;
        let key = 0xCC000000 | (source as u64) << 48 | (hue_id as u64) << 32 | art_id as u64;
        if let Some(handle) = self.texture_previews.get(&key) {
            return Some(handle.clone());
        }

        if let Some(client) = &self.client_data {
            let mut scratch = Vec::new();
            if art_id < 0x4000 {
                let mut pixels = [0u8; 44 * 44 * 4];
                if client
                    .art
                    .decode_land_tile_from_source(art_id, source, &mut scratch, &mut pixels)
                    .is_ok()
                {
                    let image = egui::ColorImage::from_rgba_unmultiplied([44, 44], &pixels[..]);
                    let handle = ctx.load_texture(
                        format!("cc_land_{}_{:?}_h{}", art_id, source, hue_id),
                        image,
                        Default::default(),
                    );
                    self.texture_previews.insert(key, handle.clone());
                    return Some(handle);
                }
            } else {
                if let Ok((w, h, mut pixels)) =
                    client
                        .art
                        .decode_static_tile_from_source(art_id, source, &mut scratch)
                {
                    // Apply hue if selected
                    if hue_id > 0 {
                        if let Some(hues) = &client.hues {
                            if let Some(hue) = hues.get((hue_id as usize).saturating_sub(1)) {
                                for i in (0..pixels.len()).step_by(4) {
                                    let r = pixels[i] as u32;
                                    let g = pixels[i + 1] as u32;
                                    let b = pixels[i + 2] as u32;
                                    let a = pixels[i + 3] as u32;
                                    let color = (a << 24) | (r << 16) | (g << 8) | b;

                                    // UO Hues are usually applied with partial_hue = true for statics?
                                    let hued = hue.apply_to_color32(color, true);

                                    pixels[i] = ((hued >> 16) & 0xFF) as u8;
                                    pixels[i + 1] = ((hued >> 8) & 0xFF) as u8;
                                    pixels[i + 2] = (hued & 0xFF) as u8;
                                    pixels[i + 3] = ((hued >> 24) & 0xFF) as u8;
                                }
                            }
                        }
                    }

                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
                        &pixels[..],
                    );
                    let handle = ctx.load_texture(
                        format!("cc_static_{}_{:?}_h{}", art_id, source, hue_id),
                        image,
                        Default::default(),
                    );
                    self.texture_previews.insert(key, handle.clone());
                    return Some(handle);
                }
            }
        }
        None
    }

    pub fn get_cc_art_texture(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
    ) -> Option<egui::TextureHandle> {
        self.get_cc_art_texture_from_source(ctx, art_id, self.selected_legacy_source)
    }
}

impl eframe::App for UopInspectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::ui::draw_ui(self, ctx);
    }
}
