//! The `effects_ec.uddp` viewer: overview, the collection / tileart / sequence frame / system / texture tables, the
//! definition tree of the selected entry, the texture view and the effect preview (simulated and rasterized on the
//! preview thread, [`PreviewWorker`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use particle_sim::hued::NO_HUE;
use udd_assets::EffectsEcPackage;
use udd_assets::effects_ec::SYSTEM_KIND_NIF;
use udd_container::UddpReader;
use uocf::enhanced::hues::{HUES_UOP_NAME, MAX_EC_HUES};

use crate::logic::effects::hues::{HueAtlas, load_hues_package, load_hues_uop, sibling_hues_package};
use crate::logic::effects::model::{
    EffectsModel, PackageVersions, RootKey, RootRow, SYSTEM_FLAG_NAMES, TEXTURE_FLAG_NAMES, TreeNode, flag_names,
    matches_filter, read_package_versions, system_kind_name, texture_source_name,
};
use crate::logic::effects::preview::{
    LoopMode, MAX_LOD_LEVEL, MAX_TIMELINE_S, MAX_ZOOM, MIN_TIMELINE_S, MIN_ZOOM, PREVIEW_STEP_MS, PartStatus,
    PreviewTarget, ResourceState, ResourceUse, SimulationSettings, ViewSettings,
};
use crate::logic::effects::texture::{DecodedLevel, checker_composite, decode_levels, format_name};
use crate::logic::effects::worker::{BuiltScene, PreviewWorker, WorkerRequest, WorkerResponse};
use crate::utils::format_size;

const TABLE_ROW_HEIGHT_EXTRA: f32 = 4.0;
const TABLE_HEADER_HEIGHT: f32 = 20.0;
const ID_COLUMN_WIDTH: f32 = 90.0;
const COUNT_COLUMN_WIDTH: f32 = 50.0;
const NAME_COLUMN_WIDTH: f32 = 180.0;
const FLAGS_COLUMN_WIDTH: f32 = 140.0;
const SECTION_SPACING: f32 = 8.0;
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 160, 60);
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 200, 120);
/// Added to the seed by "Reset": a new draw of every random choice (the golden ratio, so seeds do not repeat soon).
const RESEED_STEP: u32 = 0x9E37_79B9;
const MS_PER_SECOND: f64 = 1000.0;
/// Largest texture side drawn at 1:1 in the details panel; larger ones are scaled down to it.
const TEXTURE_VIEW_MAX_SIDE: f32 = 400.0;
/// Smallest texture side drawn (tiny mips are magnified).
const TEXTURE_VIEW_MIN_SIDE: f32 = 64.0;
/// Lowest LOD setting offered (0: below every level, plays no LOD node).
const LOD_SLIDER_MIN: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectsTab {
    Overview,
    Collection,
    Tileart,
    Frames,
    Systems,
    Textures,
}

impl EffectsTab {
    const ALL: [Self; 6] = [Self::Overview, Self::Collection, Self::Tileart, Self::Frames, Self::Systems, Self::Textures];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Collection => "Collection",
            Self::Tileart => "Tileart",
            Self::Frames => "Sequence frames",
            Self::Systems => "Systems",
            Self::Textures => "Textures",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Root(RootKey),
    System(u32),
    Texture(u32),
}

struct TextureView {
    index: u32,
    levels: Result<Vec<DecodedLevel>, String>,
    level: usize,
    show_alpha: bool,
    /// The uploaded image and what it shows (level, alpha).
    handle: Option<(usize, bool, egui::TextureHandle)>,
}

/// Where the hue atlas stands.
enum HueAtlasState {
    None,
    Loaded(HueAtlas),
    Failed(String),
}

/// The preview of the selected entry or system.
struct Preview {
    worker: Option<PreviewWorker>,
    generation: u64,
    /// What the last build request was for.
    requested: Option<(PreviewTarget, SimulationSettings)>,
    settings: SimulationSettings,
    view: ViewSettings,
    built: Option<BuiltScene>,
    error: Option<String>,
    frame: Option<egui::TextureHandle>,
    frame_info: Option<(u32, usize, bool)>,
    playing: bool,
    loop_playback: bool,
    time_ms: f64,
    last_tick: Option<f64>,
    /// A render request is out.
    awaiting_frame: bool,
    /// The last render requested: (time, view).
    rendered: Option<(u32, ViewSettings)>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            worker: None,
            generation: 0,
            requested: None,
            settings: SimulationSettings::default(),
            view: ViewSettings::default(),
            built: None,
            error: None,
            frame: None,
            frame_info: None,
            playing: true,
            loop_playback: true,
            time_ms: 0.0,
            last_tick: None,
            awaiting_frame: false,
            rendered: None,
        }
    }
}

pub struct EffectsViewer {
    path: PathBuf,
    versions: Result<PackageVersions, String>,
    /// Why the package does not open.
    open_error: Option<String>,
    package: Option<Arc<EffectsEcPackage>>,
    model: Option<EffectsModel>,
    tab: EffectsTab,
    filter: String,
    selection: Option<Selection>,
    tree: Option<(RootKey, Vec<TreeNode>)>,
    texture_view: Option<TextureView>,
    preview: Preview,
    hue_atlas: HueAtlasState,
}

impl EffectsViewer {
    /// The viewer of `reader` when it is an effects package (has `metadata/package.bin`).
    pub fn open(path: &Path, reader: &UddpReader) -> Option<Self> {
        let versions = read_package_versions(reader)?;
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            EffectsEcPackage::from_reader(reader.clone()).map(|package| {
                let model = EffectsModel::build(&package, reader);
                (package, model)
            })
        }));
        let (package, model, open_error) = match opened {
            Ok(Ok((package, model))) => (Some(Arc::new(package)), Some(model), None),
            Ok(Err(error)) => (None, None, Some(format!("{error:#}"))),
            Err(_) => (None, None, Some("the package is corrupt (reading it panicked)".to_owned())),
        };
        let hue_atlas = match sibling_hues_package(path).map(|hues| load_hues_package(&hues)) {
            Some(Ok(atlas)) => HueAtlasState::Loaded(atlas),
            Some(Err(error)) => HueAtlasState::Failed(error),
            None => HueAtlasState::None,
        };
        Some(Self {
            path: path.to_owned(),
            versions,
            open_error,
            package,
            model,
            tab: EffectsTab::Overview,
            filter: String::new(),
            selection: None,
            tree: None,
            texture_view: None,
            preview: Preview::default(),
            hue_atlas,
        })
    }

    /// Why the package does not open, if it does not.
    pub fn open_error(&self) -> Option<&str> {
        self.open_error.as_deref()
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// Answers of the preview thread; keeps the clock running while playing.
    fn poll_preview(&mut self, ctx: &egui::Context) {
        let preview = &mut self.preview;
        let Some(worker) = &preview.worker else {
            return;
        };
        for response in worker.poll() {
            match response {
                WorkerResponse::Built(built) if built.generation == preview.generation => {
                    preview.built = Some(built);
                    preview.error = None;
                }
                WorkerResponse::Frame(frame) if frame.generation == preview.generation => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
                    match &mut preview.frame {
                        Some(handle) => handle.set(image, egui::TextureOptions::LINEAR),
                        None => preview.frame = Some(ctx.load_texture("effect_preview", image, egui::TextureOptions::LINEAR)),
                    }
                    preview.frame_info = Some((frame.time_ms, frame.particles, frame.drained));
                    preview.awaiting_frame = false;
                }
                WorkerResponse::Failed { generation, error } if generation == preview.generation => {
                    preview.error = Some(error);
                    preview.awaiting_frame = false;
                }
                // An older scene's answer.
                WorkerResponse::Frame(_) => preview.awaiting_frame = false,
                _ => {}
            }
        }
        if !worker.is_running() && preview.error.is_none() {
            preview.error = Some("the preview thread stopped".to_owned());
        }
        if preview.playing {
            let now = ctx.input(|input| input.time);
            if let Some(last) = preview.last_tick {
                preview.time_ms += (now - last) * MS_PER_SECOND;
            }
            preview.last_tick = Some(now);
            let end = f64::from(preview.settings.timeline_ms());
            if preview.time_ms > end {
                if preview.loop_playback {
                    preview.time_ms %= end.max(f64::from(PREVIEW_STEP_MS));
                } else {
                    preview.time_ms = end;
                    preview.playing = false;
                }
            }
            ctx.request_repaint();
        } else {
            preview.last_tick = None;
        }
    }

    /// Requests the build and frame the UI shows for `target`.
    fn drive_preview(&mut self, ctx: &egui::Context, target: PreviewTarget) {
        let Some(package) = &self.package else {
            return;
        };
        let preview = &mut self.preview;
        if preview.worker.is_none() {
            let ctx = ctx.clone();
            let worker = PreviewWorker::spawn(Arc::clone(package), move || ctx.request_repaint());
            if let HueAtlasState::Loaded(atlas) = &self.hue_atlas {
                worker.send(WorkerRequest::SetHueAtlas(Some(Arc::clone(&atlas.texture))));
            }
            preview.worker = Some(worker);
        }
        let Some(worker) = &preview.worker else {
            return;
        };
        let wanted = (target, preview.settings);
        if preview.requested != Some(wanted) {
            if preview.requested.is_none_or(|(old, _)| old != target) {
                preview.time_ms = 0.0;
                preview.built = None;
                preview.frame_info = None;
            }
            preview.generation += 1;
            preview.frame_info = None;
            preview.requested = Some(wanted);
            preview.rendered = None;
            preview.awaiting_frame = false;
            worker.send(WorkerRequest::Build { generation: preview.generation, target, settings: preview.settings });
        }
        let time = (preview.time_ms as u32) / PREVIEW_STEP_MS * PREVIEW_STEP_MS;
        let render = (time, preview.view);
        if !preview.awaiting_frame && preview.rendered != Some(render) {
            preview.rendered = Some(render);
            preview.awaiting_frame = true;
            worker.send(WorkerRequest::Render { time_ms: time, view: preview.view });
        }
    }

    fn set_hue_atlas(&mut self, state: HueAtlasState) {
        let texture = match &state {
            HueAtlasState::Loaded(atlas) => Some(Arc::clone(&atlas.texture)),
            _ => None,
        };
        if let Some(worker) = &self.preview.worker {
            worker.send(WorkerRequest::SetHueAtlas(texture));
            // Redraw the current frame.
            self.preview.rendered = None;
        }
        self.hue_atlas = state;
    }

    fn select(&mut self, selection: Selection) {
        self.selection = Some(selection);
        self.tree = None;
        if let (Selection::Texture(index), Some(package)) = (selection, &self.package) {
            if self.texture_view.as_ref().is_none_or(|view| view.index != index) {
                let levels = package.read_texture(index).map_err(|error| format!("{error:#}")).and_then(|data| decode_levels(&data));
                self.texture_view = Some(TextureView { index, levels, level: 0, show_alpha: true, handle: None });
            }
        }
        self.preview.playing = true;
    }

    /// The central panel: version banner, tabs, filter and the table of the tab.
    pub fn ui_tables(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        self.poll_preview(ctx);
        self.ui_versions(ui);
        let Some(model) = &self.model else {
            ui.colored_label(ERROR_COLOR, "The package does not open; nothing to browse.");
            return;
        };
        ui.horizontal(|ui| {
            for tab in EffectsTab::ALL {
                let count = match tab {
                    EffectsTab::Overview => None,
                    EffectsTab::Collection => Some(model.collection.len()),
                    EffectsTab::Tileart => Some(model.tileart.len()),
                    EffectsTab::Frames => Some(model.frames.len()),
                    EffectsTab::Systems => Some(model.systems.len()),
                    EffectsTab::Textures => Some(model.textures.len()),
                };
                let label = count.map_or_else(|| tab.label().to_owned(), |count| format!("{} ({count})", tab.label()));
                ui.selectable_value(&mut self.tab, tab, label);
            }
        });
        if self.tab != EffectsTab::Overview {
            ui.horizontal(|ui| {
                ui.label("Filter (id, 0x id, name):");
                ui.text_edit_singleline(&mut self.filter);
                if ui.button("x").clicked() {
                    self.filter.clear();
                }
            });
        }
        ui.separator();
        match self.tab {
            EffectsTab::Overview => self.ui_overview(ui),
            EffectsTab::Collection | EffectsTab::Tileart | EffectsTab::Frames => self.ui_root_table(ui),
            EffectsTab::Systems => self.ui_system_table(ui),
            EffectsTab::Textures => self.ui_texture_table(ui),
        }
    }

    fn ui_versions(&self, ui: &mut egui::Ui) {
        match &self.versions {
            Ok(versions) => {
                if let Some(message) = versions.mismatch_message() {
                    ui.colored_label(ERROR_COLOR, message);
                }
            }
            Err(error) => {
                ui.colored_label(ERROR_COLOR, format!("package.bin: {error}"));
            }
        }
        if let Some(error) = &self.open_error {
            ui.colored_label(ERROR_COLOR, format!("The package does not open: {error}"));
        }
    }

    fn ui_overview(&mut self, ui: &mut egui::Ui) {
        let Some(model) = &self.model else {
            return;
        };
        let overview = &model.overview;
        let mut clicked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Package");
            crate::ui::metadata_grid("effects_versions").show(ui, |ui| {
                crate::ui::metadata_label(ui, "File:");
                ui.label(self.path.display().to_string());
                ui.end_row();
                if let Ok(versions) = &self.versions {
                    let (stored, current) = (versions.stored, versions.current);
                    let rows = [
                        ("Chaos desc version:", stored.chaos_desc_version, current.chaos_desc_version),
                        ("NIF desc version:", stored.nif_desc_version, current.nif_desc_version),
                        ("Desc codec version:", stored.desc_codec_version, current.desc_codec_version),
                    ];
                    for (label, stored, current) in rows {
                        crate::ui::metadata_label(ui, label);
                        if stored == current {
                            ui.colored_label(OK_COLOR, stored.to_string());
                        } else {
                            ui.colored_label(ERROR_COLOR, format!("{stored} (this build reads {current}: rebuild needed)"));
                        }
                        ui.end_row();
                    }
                }
            });
            ui.add_space(SECTION_SPACING);
            ui.heading("Definitions");
            crate::ui::metadata_grid("effects_definitions").show(ui, |ui| {
                let rows = [
                    ("Collection entries:", overview.collection_entries.to_string()),
                    ("Tileart tiles:", overview.tiles.to_string()),
                    (
                        "Sequence frames:",
                        format!("{} (in {} AnimationSequence entries)", overview.sequence_frames, overview.sequences),
                    ),
                    ("Definition nodes:", overview.nodes.to_string()),
                    ("Unreachable nodes:", overview.unreachable_nodes.to_string()),
                ];
                grid_rows(ui, &rows);
            });
            ui.add_space(SECTION_SPACING);
            ui.heading("Systems");
            crate::ui::metadata_grid("effects_systems").show(ui, |ui| {
                let rows = [
                    ("Systems:", format!("{} ({} Chaos, {} NIF)", overview.systems, overview.chaos, overview.nif)),
                    ("Missing from the client:", overview.missing.to_string()),
                    ("Compile failed:", overview.compile_failed.to_string()),
                    ("Compiled with warnings:", overview.warnings.to_string()),
                    ("Only used as Chaos MESH:", overview.mesh_only.to_string()),
                    ("Unreferenced:", overview.unreferenced_systems.len().to_string()),
                ];
                grid_rows(ui, &rows);
            });
            ui.add_space(SECTION_SPACING);
            ui.heading("Textures");
            crate::ui::metadata_grid("effects_textures").show(ui, |ui| {
                let counts = |map: &std::collections::BTreeMap<&str, usize>| {
                    if map.is_empty() {
                        "-".to_owned()
                    } else {
                        map.iter().map(|(name, count)| format!("{name}: {count}")).collect::<Vec<_>>().join(", ")
                    }
                };
                let rows = [
                    ("Textures:", overview.textures.to_string()),
                    ("Formats:", counts(&overview.texture_formats)),
                    ("Sources:", counts(&overview.texture_sources)),
                    ("Flags:", counts(&overview.texture_flags)),
                    (
                        "Mipmaps:",
                        format!("{} with mips, {} levels stored", overview.mipmapped_textures, overview.texture_levels),
                    ),
                    ("Unreferenced:", overview.unreferenced_textures.len().to_string()),
                ];
                grid_rows(ui, &rows);
            });
            ui.add_space(SECTION_SPACING);
            ui.label(
                egui::RichText::new(
                    "Unreferenced: no collection, tileart or sequence frame list reaches them through particles and \
                     dependencies. Files of EffectTexture.uop that no effect names are not packed; the packer \
                     report lists them.",
                )
                .weak(),
            );
            if !overview.unreferenced_systems.is_empty() {
                ui.collapsing(format!("Unreferenced systems ({})", overview.unreferenced_systems.len()), |ui| {
                    for &system in &overview.unreferenced_systems {
                        if ui.link(&model.systems[system as usize].name).clicked() {
                            clicked = Some((EffectsTab::Systems, Selection::System(system)));
                        }
                    }
                });
            }
            if !overview.unreferenced_textures.is_empty() {
                ui.collapsing(format!("Unreferenced textures ({})", overview.unreferenced_textures.len()), |ui| {
                    for &texture in &overview.unreferenced_textures {
                        if ui.link(&model.textures[texture as usize].name).clicked() {
                            clicked = Some((EffectsTab::Textures, Selection::Texture(texture)));
                        }
                    }
                });
            }
        });
        if let Some((tab, selection)) = clicked {
            self.tab = tab;
            self.select(selection);
        }
    }

    fn ui_root_table(&mut self, ui: &mut egui::Ui) {
        let Some(model) = &self.model else {
            return;
        };
        let rows: &[RootRow] = match self.tab {
            EffectsTab::Collection => &model.collection,
            EffectsTab::Tileart => &model.tileart,
            _ => &model.frames,
        };
        let visible: Vec<usize> = (0..rows.len()).filter(|&row| matches_filter(&rows[row].search, &self.filter)).collect();
        let frames = self.tab == EffectsTab::Frames;
        let row_height = egui::TextStyle::Body.resolve(ui.style()).size + TABLE_ROW_HEIGHT_EXTRA;
        let mut clicked = None;
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(ID_COLUMN_WIDTH));
        if frames {
            table = table.column(Column::auto().at_least(COUNT_COLUMN_WIDTH)).column(Column::auto().at_least(COUNT_COLUMN_WIDTH));
        }
        table
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(FLAGS_COLUMN_WIDTH))
            .column(Column::remainder())
            .header(TABLE_HEADER_HEIGHT, |mut header| {
                let id = match self.tab {
                    EffectsTab::Collection => "Collection id",
                    EffectsTab::Tileart => "Tile id",
                    _ => "Sequence",
                };
                let mut titles = vec![id];
                if frames {
                    titles.extend(["Action", "Frame"]);
                }
                titles.extend(["Roots", "Nodes", "Switches / issues", "Particles"]);
                for title in titles {
                    header.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, visible.len(), |mut table_row| {
                    let row = &rows[visible[table_row.index()]];
                    let selected = self.selection == Some(Selection::Root(row.key));
                    let id = match row.key {
                        RootKey::Collection(id) => id.to_string(),
                        RootKey::Tile(tile) => format!("{tile} ({tile:#06x})"),
                        RootKey::Frame { sequence_id, .. } => sequence_id.to_string(),
                    };
                    table_row.col(|ui| {
                        if ui.selectable_label(selected, id).clicked() {
                            clicked = Some(Selection::Root(row.key));
                        }
                    });
                    if let RootKey::Frame { action_id, frame, .. } = row.key {
                        table_row.col(|ui| {
                            ui.label(action_id.to_string());
                        });
                        table_row.col(|ui| {
                            ui.label(frame.to_string());
                        });
                    }
                    table_row.col(|ui| {
                        ui.label(row.roots.to_string());
                    });
                    table_row.col(|ui| {
                        ui.label(row.nodes.to_string());
                    });
                    table_row.col(|ui| {
                        let mut issues = row.switches.clone();
                        if row.missing > 0 {
                            issues.push(format!("{} not in client", row.missing));
                        }
                        if row.unnamed_particles > 0 {
                            issues.push(format!("{} without system", row.unnamed_particles));
                        }
                        if row.too_deep {
                            issues.push("too deep".to_owned());
                        }
                        let text = issues.join("; ");
                        if row.missing > 0 || row.too_deep {
                            ui.colored_label(WARNING_COLOR, text);
                        } else {
                            ui.label(text);
                        }
                    });
                    table_row.col(|ui| {
                        ui.label(&row.summary);
                    });
                });
            });
        if let Some(selection) = clicked {
            self.select(selection);
        }
    }

    fn ui_system_table(&mut self, ui: &mut egui::Ui) {
        let (Some(model), Some(package)) = (&self.model, &self.package) else {
            return;
        };
        let visible: Vec<usize> =
            (0..model.systems.len()).filter(|&row| matches_filter(&model.systems[row].search, &self.filter)).collect();
        let row_height = egui::TextStyle::Body.resolve(ui.style()).size + TABLE_ROW_HEIGHT_EXTRA;
        let mut clicked = None;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(NAME_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(FLAGS_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::remainder())
            .header(TABLE_HEADER_HEIGHT, |mut header| {
                for title in ["#", "Name", "Kind", "Flags", "Blob", "Users", "Dependencies"] {
                    header.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, visible.len(), |mut table_row| {
                    let row = &model.systems[visible[table_row.index()]];
                    table_row.col(|ui| {
                        ui.label(row.index.to_string());
                    });
                    table_row.col(|ui| {
                        if ui.selectable_label(self.selection == Some(Selection::System(row.index)), &row.name).clicked() {
                            clicked = Some(Selection::System(row.index));
                        }
                    });
                    table_row.col(|ui| {
                        ui.label(system_kind_name(row.kind));
                    });
                    table_row.col(|ui| {
                        let flags = flag_names(row.flags, &SYSTEM_FLAG_NAMES);
                        if row.flags & (udd_assets::effects_ec::SYSTEM_FLAG_MISSING | udd_assets::effects_ec::SYSTEM_FLAG_COMPILE_FAILED) != 0 {
                            ui.colored_label(WARNING_COLOR, flags);
                        } else {
                            ui.label(flags);
                        }
                    });
                    table_row.col(|ui| {
                        ui.label(row.blob_size.map_or_else(|| "-".to_owned(), |size| format_size(u64::from(size))));
                    });
                    table_row.col(|ui| {
                        ui.label(format!("{} / {}", row.definition_users, row.mesh_users.len()));
                    });
                    table_row.col(|ui| {
                        ui.label(row.dependency_names(package).join(", "));
                    });
                });
            });
        if let Some(selection) = clicked {
            self.select(selection);
        }
    }

    fn ui_texture_table(&mut self, ui: &mut egui::Ui) {
        let Some(model) = &self.model else {
            return;
        };
        let visible: Vec<usize> =
            (0..model.textures.len()).filter(|&row| matches_filter(&model.textures[row].search, &self.filter)).collect();
        let row_height = egui::TextStyle::Body.resolve(ui.style()).size + TABLE_ROW_HEIGHT_EXTRA;
        let mut clicked = None;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(NAME_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::auto().at_least(FLAGS_COLUMN_WIDTH))
            .column(Column::auto().at_least(COUNT_COLUMN_WIDTH))
            .column(Column::remainder())
            .header(TABLE_HEADER_HEIGHT, |mut header| {
                for title in ["#", "Name", "Size", "Format", "Mips", "Flags", "Source", "Users"] {
                    header.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, visible.len(), |mut table_row| {
                    let row = &model.textures[visible[table_row.index()]];
                    table_row.col(|ui| {
                        ui.label(row.index.to_string());
                    });
                    table_row.col(|ui| {
                        if ui.selectable_label(self.selection == Some(Selection::Texture(row.index)), &row.name).clicked() {
                            clicked = Some(Selection::Texture(row.index));
                        }
                    });
                    table_row.col(|ui| {
                        ui.label(format!("{}x{}", row.width, row.height));
                    });
                    table_row.col(|ui| {
                        ui.label(format_name(row.format));
                    });
                    table_row.col(|ui| {
                        ui.label(row.mip_count.to_string());
                    });
                    table_row.col(|ui| {
                        ui.label(flag_names(row.flags, &TEXTURE_FLAG_NAMES));
                    });
                    table_row.col(|ui| {
                        ui.label(texture_source_name(row.source));
                    });
                    table_row.col(|ui| {
                        let users: Vec<&str> = row.users.iter().map(|&user| model.systems[user as usize].name.as_str()).collect();
                        ui.label(users.join(", "));
                    });
                });
            });
        if let Some(selection) = clicked {
            self.select(selection);
        }
    }

    /// The details panel of the selection.
    pub fn ui_details(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(selection) = self.selection else {
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| match selection {
            Selection::Root(key) => {
                ui.heading(key.label());
                self.ui_preview(ctx, ui, PreviewTarget::Root(key));
                ui.add_space(SECTION_SPACING);
                self.ui_tree(ui, key);
            }
            Selection::System(system) => {
                self.ui_system_details(ui, system);
                ui.add_space(SECTION_SPACING);
                self.ui_preview(ctx, ui, PreviewTarget::System(system));
            }
            Selection::Texture(_) => self.ui_texture_details(ctx, ui),
        });
    }

    fn ui_system_details(&mut self, ui: &mut egui::Ui, system: u32) {
        let (Some(model), Some(package)) = (&self.model, &self.package) else {
            return;
        };
        let Some(row) = model.systems.get(system as usize) else {
            return;
        };
        ui.heading(&row.name);
        let mut clicked = None;
        crate::ui::metadata_grid("effects_system_details").show(ui, |ui| {
            let rows = [
                ("Index:", row.index.to_string()),
                ("Kind:", system_kind_name(row.kind).to_owned()),
                ("Flags:", flag_names(row.flags, &SYSTEM_FLAG_NAMES)),
                ("Blob:", row.blob_size.map_or_else(|| "none".to_owned(), |size| format_size(u64::from(size)))),
                ("Particle nodes:", row.definition_users.to_string()),
            ];
            grid_rows(ui, &rows);
            crate::ui::metadata_label(ui, "Used as MESH by:");
            ui.vertical(|ui| {
                for &user in &row.mesh_users {
                    if ui.link(&model.systems[user as usize].name).clicked() {
                        clicked = Some(Selection::System(user));
                    }
                }
            });
            ui.end_row();
            crate::ui::metadata_label(ui, "Dependencies:");
            ui.vertical(|ui| {
                for &dep in &row.deps {
                    let target = dep & !udd_assets::effects_ec::DEP_SYSTEM_BIT;
                    let name = crate::logic::effects::model::dependency_name(package, dep);
                    if ui.link(name).clicked() {
                        clicked = Some(if dep & udd_assets::effects_ec::DEP_SYSTEM_BIT != 0 {
                            Selection::System(target)
                        } else {
                            Selection::Texture(target)
                        });
                    }
                }
            });
            ui.end_row();
        });
        if row.kind == SYSTEM_KIND_NIF {
            ui.label(egui::RichText::new("Previewed as a particle with default fields and hued set.").weak());
        }
        if let Some(selection) = clicked {
            self.tab = match selection {
                Selection::Texture(_) => EffectsTab::Textures,
                _ => EffectsTab::Systems,
            };
            self.select(selection);
        }
    }

    fn ui_texture_details(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let (Some(model), Some(view)) = (&self.model, &mut self.texture_view) else {
            return;
        };
        let Some(row) = model.textures.get(view.index as usize) else {
            return;
        };
        ui.heading(&row.name);
        crate::ui::metadata_grid("effects_texture_details").show(ui, |ui| {
            let users: Vec<&str> = row.users.iter().map(|&user| model.systems[user as usize].name.as_str()).collect();
            let rows = [
                ("Index:", row.index.to_string()),
                ("Size:", format!("{}x{}", row.width, row.height)),
                ("Format:", format_name(row.format).to_owned()),
                ("Mip levels:", row.mip_count.to_string()),
                ("Flags:", flag_names(row.flags, &TEXTURE_FLAG_NAMES)),
                ("Source:", texture_source_name(row.source).to_owned()),
                ("Payload:", row.payload_size.map_or_else(|| "none".to_owned(), |size| format_size(u64::from(size)))),
                ("Used by:", if users.is_empty() { "-".to_owned() } else { users.join(", ") }),
            ];
            grid_rows(ui, &rows);
        });
        ui.add_space(SECTION_SPACING);
        let levels = match &view.levels {
            Ok(levels) if !levels.is_empty() => levels,
            Ok(_) => {
                ui.label("No payload: the client lacks the file, or it does not decode (see flags). Drawn white.");
                return;
            }
            Err(error) => {
                ui.colored_label(ERROR_COLOR, format!("Does not decode: {error}"));
                return;
            }
        };
        ui.horizontal(|ui| {
            ui.label("Mip level:");
            let last = levels.len() - 1;
            ui.add(egui::Slider::new(&mut view.level, 0..=last));
            ui.checkbox(&mut view.show_alpha, "Alpha over checker");
        });
        let level = &levels[view.level.min(levels.len() - 1)];
        ui.label(format!("Level {}: {}x{}", view.level, level.width, level.height));
        let stale = view.handle.as_ref().is_none_or(|(shown, alpha, _)| *shown != view.level || *alpha != view.show_alpha);
        if stale {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [level.width as usize, level.height as usize],
                &checker_composite(level, view.show_alpha),
            );
            let handle = ctx.load_texture("effect_texture", image, egui::TextureOptions::NEAREST);
            view.handle = Some((view.level, view.show_alpha, handle));
        }
        if let Some((_, _, handle)) = &view.handle {
            let size = egui::vec2(level.width as f32, level.height as f32);
            let longest = size.x.max(size.y).max(1.0);
            let scale = (TEXTURE_VIEW_MAX_SIDE / longest).min(1.0).max(TEXTURE_VIEW_MIN_SIDE / longest);
            ui.image((handle.id(), size * scale));
        }
    }

    fn ui_preview(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, target: PreviewTarget) {
        if self.package.is_none() {
            return;
        }
        self.drive_preview(ctx, target);
        let hue_cases: Vec<u32> = match (target, &self.model) {
            (PreviewTarget::Root(key), Some(model)) => model
                .collection
                .iter()
                .chain(&model.tileart)
                .chain(&model.frames)
                .find(|row| row.key == key)
                .map(|row| row.hue_cases.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        ui.add_space(SECTION_SPACING);
        ui.strong("Preview");
        let preview = &mut self.preview;
        let end_ms = preview.settings.timeline_ms();
        ui.horizontal(|ui| {
            if ui.button(if preview.playing { "Pause" } else { "Play" }).clicked() {
                preview.playing = !preview.playing;
                if preview.playing && preview.time_ms >= f64::from(end_ms) {
                    preview.time_ms = 0.0;
                }
            }
            if ui.button("Reset").on_hover_text("Restart with a new seed (new random draws)").clicked() {
                preview.settings.seed = preview.settings.seed.wrapping_add(RESEED_STEP);
                preview.time_ms = 0.0;
            }
            ui.checkbox(&mut preview.loop_playback, "Loop playback");
            ui.label(format!("seed {:#x}", preview.settings.seed));
        });
        let mut time = preview.time_ms as u32;
        let slider = ui.add(egui::Slider::new(&mut time, 0..=end_ms).text("ms"));
        if slider.changed() {
            preview.time_ms = f64::from(time);
            preview.playing = false;
        }
        ui.horizontal(|ui| {
            ui.label("Particles loop:");
            egui::ComboBox::from_id_salt("effects_loop_mode").selected_text(preview.settings.loop_mode.label()).show_ui(ui, |ui| {
                for mode in LoopMode::ALL {
                    ui.selectable_value(&mut preview.settings.loop_mode, mode, mode.label());
                }
            });
            ui.label("Timeline s:");
            ui.add(egui::DragValue::new(&mut preview.settings.timeline_s).range(MIN_TIMELINE_S..=MAX_TIMELINE_S).speed(0.1));
        });
        ui.horizontal(|ui| {
            ui.label("LOD:");
            ui.add(egui::Slider::new(&mut preview.settings.lod_level, LOD_SLIDER_MIN..=MAX_LOD_LEVEL));
            if matches!(target, PreviewTarget::Root(RootKey::Tile(_))) {
                ui.checkbox(&mut preview.settings.play_projectile_only, "Play attach mode 2");
            }
        });
        ui.horizontal(|ui| {
            ui.label("Object hue:");
            ui.add(egui::DragValue::new(&mut preview.settings.hue).range(0..=u32::from(MAX_EC_HUES)));
            if preview.settings.hue != NO_HUE {
                let name = match &self.hue_atlas {
                    HueAtlasState::Loaded(atlas) => atlas.hue_name(preview.settings.hue).unwrap_or("(unnamed)"),
                    _ => "",
                };
                ui.label(name);
            }
            if ui.small_button("no hue").clicked() {
                preview.settings.hue = NO_HUE;
            }
        });
        if !hue_cases.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("PropertyFlag hue cases:");
                for case in &hue_cases {
                    ui.selectable_value(&mut preview.settings.hue, *case, case.to_string());
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            ui.add(egui::Slider::new(&mut preview.view.zoom, MIN_ZOOM..=MAX_ZOOM).logarithmic(true));
            ui.checkbox(&mut preview.view.show_emitter, "Anchor");
        });
        let mut atlas_change = None;
        ui.horizontal_wrapped(|ui| {
            match &self.hue_atlas {
                HueAtlasState::Loaded(atlas) => ui.label(format!("Hue atlas: {}", atlas.source.display())),
                HueAtlasState::None => ui.colored_label(WARNING_COLOR, "No hue atlas: hued parts are drawn without hue."),
                HueAtlasState::Failed(error) => ui.colored_label(ERROR_COLOR, format!("Hue atlas: {error}")),
            };
            let dir = self.path.parent();
            if ui.small_button("hues.uddp...").clicked() {
                if let Some(path) = uot_gui::open_filtered_file_dialog(dir, "Hues package", &["uddp"]) {
                    atlas_change = Some(load_hues_package(&path));
                }
            }
            if ui.small_button(format!("{HUES_UOP_NAME}...")).clicked() {
                if let Some(path) = uot_gui::open_filtered_file_dialog(dir, "Hues.uop", &["uop"]) {
                    atlas_change = Some(load_hues_uop(&path));
                }
            }
        });
        if let Some(change) = atlas_change {
            self.set_hue_atlas(match change {
                Ok(atlas) => HueAtlasState::Loaded(atlas),
                Err(error) => HueAtlasState::Failed(error),
            });
        }
        let preview = &self.preview;
        if let Some(error) = &preview.error {
            ui.colored_label(ERROR_COLOR, error);
        }
        if let Some(handle) = &preview.frame {
            let side = preview.view.resolution_px as f32;
            ui.image((handle.id(), egui::vec2(side, side)));
        }
        match (&preview.built, preview.frame_info) {
            (Some(built), Some((time_ms, particles, drained))) => {
                let state = if drained { ", drained" } else { "" };
                ui.label(format!("{time_ms} ms, {particles} particles, {} part(s) playing{state}", built.playing_parts));
            }
            (None, _) if preview.error.is_none() => {
                ui.label("Building...");
            }
            _ => {}
        }
        if let Some(built) = &preview.built {
            if built.has_hued_parts && !matches!(self.hue_atlas, HueAtlasState::Loaded(_)) {
                ui.colored_label(WARNING_COLOR, "Hued parts play without hue: load hues.uddp or Hues.uop.");
            }
            ui.collapsing(format!("Parts ({})", built.parts.len()), |ui| {
                for part in &built.parts {
                    let text = format!("[{}] {}", part.path, part.status.describe());
                    match &part.status {
                        status if status.plays() => ui.colored_label(OK_COLOR, text),
                        PartStatus::NotVisual { .. } | PartStatus::ProjectileOnly { .. } => ui.label(text),
                        _ => ui.colored_label(WARNING_COLOR, text),
                    };
                    ui.indent(("part", &part.path), |ui| {
                        for (kind, resources) in [("mesh", &part.meshes), ("tex", &part.textures)] {
                            for resource in resources {
                                resource_label(ui, kind, resource);
                            }
                        }
                    });
                }
            });
            if !built.notes.is_empty() {
                ui.collapsing(format!("Choices ({})", built.notes.len()), |ui| {
                    for note in &built.notes {
                        ui.label(note);
                    }
                });
            }
        }
    }

    fn ui_tree(&mut self, ui: &mut egui::Ui, key: RootKey) {
        let Some(package) = &self.package else {
            return;
        };
        if self.tree.as_ref().is_none_or(|(shown, _)| *shown != key) {
            let roots = key.roots(package);
            self.tree = Some((key, crate::logic::effects::model::definition_tree(package, roots)));
        }
        let Some((_, tree)) = &self.tree else {
            return;
        };
        ui.strong(format!("Definition ({} root(s))", tree.len()));
        for node in tree {
            tree_node_ui(ui, node, "root");
        }
    }
}

fn grid_rows(ui: &mut egui::Ui, rows: &[(&str, String)]) {
    for (label, value) in rows {
        crate::ui::metadata_label(ui, *label);
        ui.label(value);
        ui.end_row();
    }
}

fn resource_label(ui: &mut egui::Ui, kind: &str, resource: &ResourceUse) {
    let (text, color) = match &resource.state {
        ResourceState::Loaded => ("loaded".to_owned(), None),
        ResourceState::NoPayload { flags } => {
            (format!("no payload ({}), drawn white / outlined", flag_names(*flags, &TEXTURE_FLAG_NAMES)), Some(WARNING_COLOR))
        }
        ResourceState::Failed(error) => (format!("failed: {error}"), Some(ERROR_COLOR)),
        ResourceState::NotInPackage => ("not in the package".to_owned(), Some(ERROR_COLOR)),
        ResourceState::NotRenamed => ("embedded pixels without a package name".to_owned(), Some(ERROR_COLOR)),
    };
    let text = format!("{kind} {}: {text}", resource.name);
    match color {
        Some(color) => ui.colored_label(color, text),
        None => ui.label(text),
    };
}

fn tree_node_ui(ui: &mut egui::Ui, node: &TreeNode, parent: &str) {
    let id = format!("{parent}/{}", node.node);
    egui::CollapsingHeader::new(&node.title).id_salt(&id).default_open(true).show(ui, |ui| {
        egui::Grid::new(("params", &id)).num_columns(2).striped(true).show(ui, |ui| {
            for (name, value) in &node.params {
                crate::ui::metadata_label(ui, *name);
                ui.label(value);
                ui.end_row();
            }
        });
        for group in &node.groups {
            egui::CollapsingHeader::new(format!("{} ({})", group.label, group.children.len()))
                .id_salt((&id, &group.label))
                .default_open(true)
                .show(ui, |ui| {
                    for child in &group.children {
                        tree_node_ui(ui, child, &format!("{id}/{}", group.label));
                    }
                });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::effects::test_package::{CASE_HUE, TestPackage, ids, systems, textures};
    use udd_assets::effects_ec::PackageRecord;

    /// The viewer does not read the file (only looks for `hues.uddp` beside it).
    const FAKE_PATH: &str = "/nonexistent/effects_ec.uddp";
    /// Frames run while waiting for the preview thread.
    const MAX_FRAMES: usize = 500;
    const FRAME_WAIT: std::time::Duration = std::time::Duration::from_millis(5);

    fn viewer(package: TestPackage) -> EffectsViewer {
        let reader = UddpReader::open(package.into_bytes()).unwrap();
        EffectsViewer::open(Path::new(FAKE_PATH), &reader).expect("an effects package")
    }

    fn frame(ctx: &egui::Context, viewer: &mut EffectsViewer) {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            viewer.ui_tables(ctx, ui);
            viewer.ui_details(ctx, ui);
        });
    }

    /// Runs frames until the preview shows a frame of the current scene.
    fn until_previewed(ctx: &egui::Context, viewer: &mut EffectsViewer) {
        for _ in 0..MAX_FRAMES {
            frame(ctx, viewer);
            let current = viewer.preview.built.as_ref().is_some_and(|built| built.generation == viewer.preview.generation);
            if current && viewer.preview.frame_info.is_some() {
                return;
            }
            std::thread::sleep(FRAME_WAIT);
        }
        panic!("no preview: {:?}", viewer.preview.error);
    }

    #[test]
    fn every_tab_and_selection_draws() {
        let ctx = egui::Context::default();
        let mut viewer = viewer(TestPackage::standard());
        assert!(viewer.open_error().is_none());
        for tab in EffectsTab::ALL {
            viewer.tab = tab;
            frame(&ctx, &mut viewer);
        }
        let selections = [
            (EffectsTab::Collection, Selection::Root(RootKey::Collection(ids::HUE_SWITCH))),
            (EffectsTab::Tileart, Selection::Root(RootKey::Tile(ids::TILE))),
            (
                EffectsTab::Frames,
                Selection::Root(RootKey::Frame { sequence_id: ids::SEQUENCE, action_id: ids::ACTION, frame: ids::FRAME }),
            ),
            (EffectsTab::Systems, Selection::System(systems::MESH)),
        ];
        for (tab, selection) in selections {
            viewer.tab = tab;
            viewer.select(selection);
            until_previewed(&ctx, &mut viewer);
        }
        // A hue case rebuilds the scene with the hued NIF.
        viewer.select(Selection::Root(RootKey::Collection(ids::HUE_SWITCH)));
        viewer.preview.settings.hue = CASE_HUE;
        until_previewed(&ctx, &mut viewer);
        let built = viewer.preview.built.as_ref().unwrap();
        assert!(built.has_hued_parts, "{:?}", built.parts);

        viewer.tab = EffectsTab::Textures;
        viewer.select(Selection::Texture(textures::SPARK));
        frame(&ctx, &mut viewer);
        let view = viewer.texture_view.as_mut().unwrap();
        assert_eq!(view.levels.as_ref().map(Vec::len), Ok(2));
        view.level = 1;
        frame(&ctx, &mut viewer);
        assert!(viewer.texture_view.as_ref().unwrap().handle.as_ref().is_some_and(|(level, _, _)| *level == 1));
        viewer.select(Selection::Texture(textures::GONE));
        frame(&ctx, &mut viewer);
    }

    #[test]
    fn a_package_of_other_versions_shows_the_error() {
        let other = PackageRecord { nif_desc_version: PackageRecord::CURRENT.nif_desc_version + 1, ..PackageRecord::CURRENT };
        let mut viewer = viewer(TestPackage { versions: other, ..TestPackage::standard() });
        assert!(viewer.open_error().is_some_and(|error| error.contains("repack")));
        assert!(viewer.versions.as_ref().unwrap().mismatch_message().is_some());
        frame(&egui::Context::default(), &mut viewer);
    }
}
