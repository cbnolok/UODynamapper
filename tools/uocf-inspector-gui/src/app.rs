use crate::logic::{ClientData, Dictionary, UopCache};
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;
use uocf::classic::art::ArtMap;
pub use uocf::classic::art::ArtSource;
use uocf::classic::tiledata::TileData;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::enhanced::terrain_definition::TerrainDefinitionEntry;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::uop_container::package::{LoadMode, UopPackage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ViewMode {
    Home,
    UopExplorer,
    TexArtCc,
    CcTileData,
    TileMetadata,
    Animations,
    Multis,
    Hues,
    TerrainDefinition,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum TileMetadataSource {
    CcTileData,
    EcTileArt,
}

#[derive(Clone)]
pub struct TerrainDefinitionFileEntry {
    pub filename_hash: u64,
    pub byte_len: usize,
    pub raw_prefix: Vec<u8>,
    pub entry: TerrainDefinitionEntry,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppSettings {
    pub cc_path: Option<PathBuf>,
    pub ec_path: Option<PathBuf>,
    pub dict_path: Option<PathBuf>,
    pub last_view_mode: Option<ViewMode>,
}

pub struct UopInspectorApp {
    pub settings: AppSettings,
    pub logs: Vec<String>,
    pub show_search_paths: bool,

    pub dictionary: Dictionary,
    pub uo_string_dictionary: Option<Arc<UoStringDictionary>>,
    pub uop_cache: UopCache,
    pub client_data: Option<ClientData>,
    pub cc_tiledata: Option<Arc<TileData>>,

    pub selected_uop_idx: Option<usize>,
    pub selected_file_hash: Option<u64>,
    pub selected_tex_art_cc_id: Option<u32>,
    pub selected_terrain_def_hash: Option<u64>,
    // pub selected_cc_tile_id: Option<u32>,
    pub selected_legacy_source: ArtSource,

    pub search_query: String,
    pub find_hash_query: String,
    pub status_message: String,
    pub view_mode: ViewMode,
    pub tile_metadata_source: TileMetadataSource,

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

    pub terrain_def_package: Option<Arc<uocf::enhanced::terrain_definition::TerrainDefinitionPackage>>,
    pub terrain_def_files: Option<Arc<Vec<TerrainDefinitionFileEntry>>>,
    pub ec_tileart_entries: Option<Arc<Vec<TileArtEntry>>>,
}

impl UopInspectorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        color_eyre::install().ok();

        let settings: AppSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "uocf_inspector_settings"))
            .unwrap_or_default();

        let mut app = Self {
            settings: settings.clone(),
            logs: Vec::new(),
            show_search_paths: false,

            dictionary: Dictionary::new(),
            uo_string_dictionary: None,
            uop_cache: UopCache::new(),
            client_data: None,
            cc_tiledata: None,
            selected_uop_idx: None,
            selected_file_hash: None,
            selected_tex_art_cc_id: None,
            selected_terrain_def_hash: None,
            // selected_cc_tile_id: None,
            selected_legacy_source: ArtSource::Any,
            search_query: String::new(),
            find_hash_query: String::new(),
            status_message: "Welcome to UOCF Inspector".to_string(),
            view_mode: settings.last_view_mode.unwrap_or(ViewMode::Home),
            tile_metadata_source: TileMetadataSource::CcTileData,
            terrain_def_package: None,
            terrain_def_files: None,
            ec_tileart_entries: None,
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
        };

        app.log("UOCF Inspector starting...");
        app.log("Loading previous settings...");
        app.trigger_reload();

        app
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        println!("{}", msg);
        self.logs.push(msg);
    }

    pub fn trigger_reload(&mut self) {
        self.log("Starting asset reload...");

        // 1. Try to load CC assets (mul and uop)
        if let Some(path) = self.settings.cc_path.clone() {
            self.log(format!("Trying to load CC assets from {}", path.display()));
            let art_res = ArtMap::load(&path);
            let td_res = TileData::load(path.join("tiledata.mul"));
            let loaded_tiledata = match td_res {
                Ok(td) => {
                    let td = Arc::new(td);
                    self.cc_tiledata = Some(Arc::clone(&td));
                    Some(td)
                }
                Err(e) => {
                    self.log(format!("tiledata.mul unavailable: {}", e));
                    self.cc_tiledata = None;
                    None
                }
            };

            match art_res {
                Ok(art) => {
                    let td = loaded_tiledata.unwrap_or_else(|| Arc::new(TileData::new_empty()));
                    let multis = uocf::classic::multi::MultiMap::load(&path)
                        .ok()
                        .map(Arc::new);
                    let hues = uocf::classic::hues::load_hues(&path.join("hues.mul"))
                        .ok()
                        .map(Arc::new);

                    let mut anim_defs = None;
                    let anim_def_path = path.join("AnimationDefinition.uop");
                    if anim_def_path.exists() {
                        if let Ok(package) = UopPackage::load(&anim_def_path) {
                            let hash = uocf::uop_container::hash::hash_file_name_single(
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

                    self.client_data = Some(ClientData {
                        path: path.clone(),
                        art: Arc::new(art),
                        tiledata: td,
                        multis,
                        _ec_multis: None,
                        hues,
                        anim_defs,
                    });
                    self.log("Successfully loaded CC assets.");
                }
                Err(e) => {
                    self.log(format!("Failed to load CC art assets: {}", e));
                }
            }
        }

        // 2. Try to load EC assets (string dictionary and legacy texture)
        if let Some(ec_base_path) = self.settings.ec_path.clone() {
            self.log(format!(
                "Trying to load EC assets from {}",
                ec_base_path.display()
            ));

            // Try load string dictionary
            let sd_path = ec_base_path.join("string_dictionary.uop");
            if sd_path.exists() {
                self.log(format!("Loading string dictionary from {}", sd_path.display()));
                match UoStringDictionary::load(&sd_path) {
                    Ok(dict) => {
                        self.uo_string_dictionary = Some(Arc::new(dict));
                        self.log("Successfully loaded EC string dictionary.");
                    }
                    Err(e) => {
                        self.log(format!("Failed to load EC string dictionary: {}", e));
                    }
                }
            }

            // Try load LegacyTexture.uop and attach to CC art if available
            let lt_path = ec_base_path.join("LegacyTexture.uop");
            if lt_path.exists() {
                self.log(format!("Loading LegacyTexture.uop from {}", lt_path.display()));
                match UopPackage::load(&lt_path) {
                    Ok(package) => {
                        if let Some(client) = &mut self.client_data {
                            let mut art = (*client.art).clone();
                            art = art.with_uop(package.clone());
                            client.art = Arc::new(art);
                            self.log("Attached LegacyTexture.uop to CC art.");
                        } else {
                            // If no CC path, load it standalone
                            let art = ArtMap::load_standalone_uop(package.clone());
                            self.client_data = Some(ClientData {
                                path: ec_base_path.clone(),
                                art: Arc::new(art),
                                tiledata: Arc::new(TileData::new_empty()),
                                multis: None,
                                _ec_multis: None,
                                hues: None,
                                anim_defs: None,
                            });
                            self.log("Loaded standalone Legacy Art UOP from EC folder.");
                        }
                    }
                    Err(e) => {
                        self.log(format!("Failed to load LegacyTexture.uop: {}", e));
                    }
                }
            }

            // Load additional EC UOPs into the cache for exploration
            let ec_uops = [
                "tileart.uop",
                "terraindefinition.uop",
                "terraintexture.uop",
                "legacytexture.uop",
                "legacyterrain.uop",
                "texture.uop",
            ];
            for uop_name in ec_uops {
                let uop_path = ec_base_path.join(uop_name);
                if uop_path.exists() {
                    self.log(format!("Loading {} into cache", uop_name));
                    match UopPackage::load(&uop_path) {
                        Ok(package) => {
                            self.uop_cache.loaded_uops.push(Arc::new(crate::logic::uop_cache::LoadedUop {
                                path: uop_path,
                                package,
                            }));
                            self.log(format!("Successfully loaded {}", uop_name));
                        }
                        Err(e) => {
                            self.log(format!("Failed to load {}: {}", uop_name, e));
                        }
                    }
                }
            }

            let tileart_path = ec_base_path.join("tileart.uop");
            if tileart_path.exists() {
                self.log(format!("Parsing tileart.uop from {}", tileart_path.display()));
                match UopPackage::load(&tileart_path) {
                    Ok(package) => {
                        let mut entries = Vec::new();
                        let mut failed = 0usize;
                        for file in package.iter_files() {
                            match TileArtEntry::parse_raw(&file) {
                                Ok(entry) => entries.push(entry),
                                Err(_) => failed += 1,
                            }
                        }
                        entries.sort_by_key(|entry| entry.tile_id);
                        let count = entries.len();
                        self.ec_tileart_entries = Some(Arc::new(entries));
                        self.log(format!(
                            "Parsed {} tileart.uop entries ({} skipped).",
                            count, failed
                        ));
                    }
                    Err(e) => {
                        self.log(format!("Failed to load tileart.uop: {}", e));
                    }
                }
            }

            // Also load TerrainDefinitionPackage for the ad-hoc viewer
            let td_path = ec_base_path.join("terraindefinition.uop");
            if td_path.exists() {
                self.log(format!(
                    "Parsing TerrainDefinitionPackage from {}",
                    td_path.display()
                ));
                match UopPackage::load(&td_path) {
                    Ok(package) => {
                        let dict_arc = self.uo_string_dictionary.clone();
                        let dict = dict_arc.as_deref();
                        match uocf::enhanced::terrain_definition::TerrainDefinitionPackage::from_package(
                            &package,
                            dict,
                        ) {
                            Ok(pkg) => {
                                self.terrain_def_package = Some(Arc::new(pkg));
                                self.log("Successfully parsed TerrainDefinitionPackage.");
                            }
                            Err(e) => {
                                self.log(format!("Failed to parse TerrainDefinitionPackage: {}", e));
                            }
                        }

                        let mut files = Vec::new();
                        let mut failed = 0usize;
                        for file in package.iter_files() {
                            if !file.has_size() {
                                continue;
                            }

                            let data = match file.unpack() {
                                Ok(data) => data,
                                Err(_) => {
                                    failed += 1;
                                    continue;
                                }
                            };
                            match uocf::enhanced::terrain_definition::parse_entry(&file, dict) {
                                Ok(entry) => {
                                    files.push(TerrainDefinitionFileEntry {
                                        filename_hash: file.filename_hash(),
                                        byte_len: data.len(),
                                        raw_prefix: data[..data.len().min(256)].to_vec(),
                                        entry,
                                    });
                                }
                                Err(_) => failed += 1,
                            }
                        }
                        files.sort_by_key(|file| file.entry.id);
                        let count = files.len();
                        self.terrain_def_files = Some(Arc::new(files));
                        self.log(format!(
                            "Parsed {} TerrainDefinition.uop files ({} skipped).",
                            count, failed
                        ));
                    }
                    Err(e) => {
                        self.log(format!("Failed to load TerrainDefinition.uop: {}", e));
                    }
                }
            }
        }

        // 3. Try to load Dictionary from settings path
        if let Some(path) = self.settings.dict_path.clone() {
            self.log(format!(
                "Trying to load dictionary from {}",
                path.display()
            ));
            match self.dictionary.load_bin(&path) {
                Ok(_) => {
                    self.log(format!(
                        "Loaded binary dictionary with {} entries",
                        self.dictionary.count()
                    ));
                }
                Err(e) => {
                    self.log(format!("Failed to load binary dictionary: {}", e));
                }
            }
        }

        // 4. Always try to find .bin dicts in current folder as well
        self.load_local_dictionaries();
    }

    fn load_local_dictionaries(&mut self) {
        // Try current dir
        self.scan_dir_for_dicts(".");

        // Try exe dir
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(dir) = exe_path.parent() {
                if dir != std::path::Path::new(".") {
                    self.scan_dir_for_dicts(dir);
                }
            }
        }
    }

    fn scan_dir_for_dicts(&mut self, dir: impl AsRef<std::path::Path>) {
        let dir = dir.as_ref();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("bin") {
                    self.log(format!("Found binary dictionary candidate: {}", path.display()));
                    match self.dictionary.load_bin(&path) {
                        Ok(_) => {
                            self.log(format!(
                                "Successfully loaded binary dictionary: {}",
                                path.display()
                            ));
                        }
                        Err(_) => {
                            // Silently ignore if it's not a valid dict, might be another .bin file
                        }
                    }
                }
            }
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
                let hash = uocf::uop_container::hash::hash_file_name_single(&path);
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

    fn get_uop_art_image_texture(
        &mut self,
        ctx: &egui::Context,
        key: u64,
        art_id: u32,
        source: ArtSource,
        scratch: &[u8],
    ) -> Option<egui::TextureHandle> {
        let format = if scratch.starts_with(b"DDS ") {
            ECImageFormat::DDS
        } else {
            ECImageFormat::TGA
        };
        let tex_file = TextureFile {
            metadata: RawTextureItem::absent(),
            is_ec: source == ArtSource::EcUop,
            format,
            props: None,
            raw_data: Arc::from(scratch),
        };
        let img = tex_file.decode_to_rgba().ok()?;
        let rgba = img.to_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        let handle = ctx.load_texture(
            format!("uop_art_image_{}_{:?}_h{}", art_id, source, self.selected_hue_id),
            image,
            Default::default(),
        );
        self.texture_previews.insert(key, handle.clone());
        Some(handle)
    }

    fn get_tex_art_cc_texture_from_source(
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
            let art = Arc::clone(&client.art);
            let hues = client.hues.clone();
            let mut scratch = Vec::new();
            if art_id < 0x4000 {
                if art
                    .get_raw_art_data_from_source(art_id, source, &mut scratch)
                    .is_ok()
                {
                    if scratch.starts_with(b"DDS ") {
                        return self.get_uop_art_image_texture(ctx, key, art_id, source, &scratch);
                    }
                    if source != ArtSource::Mul {
                        if let Some(handle) =
                            self.get_uop_art_image_texture(ctx, key, art_id, source, &scratch)
                        {
                            return Some(handle);
                        }
                    }

                    let mut pixels = [0u8; 44 * 44 * 4];
                    if uocf::classic::art::decode_land_tile_from_raw(&scratch, &mut pixels)
                        .is_ok()
                    {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied([44, 44], &pixels[..]);
                        let handle = ctx.load_texture(
                            format!("cc_land_{}_{:?}_h{}", art_id, source, hue_id),
                            image,
                            Default::default(),
                        );
                        self.texture_previews.insert(key, handle.clone());
                        return Some(handle);
                    }
                }
            } else {
                if art
                    .get_raw_art_data_from_source(art_id, source, &mut scratch)
                    .is_ok()
                    && scratch.starts_with(b"DDS ")
                {
                    return self.get_uop_art_image_texture(ctx, key, art_id, source, &scratch);
                }

                if let Ok((w, h, mut pixels)) =
                    art.decode_static_tile_from_source(art_id, source, &mut scratch)
                {
                    // Apply hue if selected
                    if hue_id > 0 {
                        if let Some(hues) = &hues {
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

    pub fn get_tex_art_cc_texture(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
    ) -> Option<egui::TextureHandle> {
        self.get_tex_art_cc_texture_from_source(ctx, art_id, self.selected_legacy_source)
    }
}

impl eframe::App for UopInspectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::ui::draw_ui(self, ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.last_view_mode = Some(self.view_mode);
        eframe::set_value(storage, "uocf_inspector_settings", &self.settings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uocf_inspector_app_manual_log() {
        let mut app = UopInspectorApp {
            settings: AppSettings::default(),
            logs: Vec::new(),
            show_search_paths: false,
            dictionary: Dictionary::new(),
            uo_string_dictionary: None,
            uop_cache: UopCache::new(),
            client_data: None,
            cc_tiledata: None,
            selected_uop_idx: None,
            selected_file_hash: None,
            selected_tex_art_cc_id: None,
            selected_terrain_def_hash: None,
            selected_legacy_source: ArtSource::Any,
            search_query: String::new(),
            find_hash_query: String::new(),
            status_message: String::new(),
            view_mode: ViewMode::Home,
            tile_metadata_source: TileMetadataSource::CcTileData,
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
            terrain_def_package: None,
            terrain_def_files: None,
            ec_tileart_entries: None,
        };

        assert_eq!(app.logs.len(), 0);
        app.log("hello test");
        assert_eq!(app.logs.len(), 1);
        assert_eq!(app.logs[0], "hello test");
    }
}
