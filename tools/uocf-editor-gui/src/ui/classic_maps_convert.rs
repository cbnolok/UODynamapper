//! The Convert and save section of the Maps tab: a whole facet from a Classic folder or a facet package to Classic MUL,
//! Classic UOP, an Enhanced Client facet or a Kingdom Reborn facet, through [`crate::logic::facet_convert`]. It writes to
//! a folder the user picks at once, so it holds nothing unsaved.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use uocf::classic::facet_image_render::FacetImageMode;
use uocf::classic::facet_sources::CLASSIC_FACET_NAMES;
use uocf::classic::map::{MapSizeCells, FELUCCA_HEIGHT, ML_FELUCCA_WIDTH, PRE_ML_FELUCCA_WIDTH};
use uocf::classic::world::{LandFill, VOID_LAND_ID, VOID_LAND_Z};
use uocf::client::facet_dimensions::{FACET_SIDE_ALIGNMENT, MAX_FACET_SIDE};
use uocf::map_edition::MapEdition;

use crate::logic::facet_convert::{convert_facet, package_name_facet, ConvertRequest, DifChoice, FacetSource, FacetTarget};

const PACKAGE_FILTER: (&str, &[&str]) = ("Facet package", &["uop"]);
const CLIENT_FILTER: (&str, &[&str]) = ("Client executable", &["exe"]);
/// How often the tab looks at a running conversion.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const LAND_HEX_DIGITS: usize = 4;
/// Sizes the shipped clients give facets 0 and 1 (`docs/agent_notes/cc_file_coverage.md`): the older one is the size of
/// the clients before Mondain's Legacy.
const SIZE_PRESETS: [(&str, MapSizeCells); 2] = [
    ("Felucca, Trammel of the old clients: 6144 x 4096", MapSizeCells { width: PRE_ML_FELUCCA_WIDTH, height: FELUCCA_HEIGHT }),
    ("Felucca, Trammel: 7168 x 4096", MapSizeCells { width: ML_FELUCCA_WIDTH, height: FELUCCA_HEIGHT }),
];
const AUTOMATIC_SIZE_TEXT: &str = "The target client's, else the source's";
const CUSTOM_SIZE_TEXT: &str = "Custom";
const KR_FALLBACK_HOVER: &str = "Stores the material and subtype of this Classic land id for the land ids Kingdom Reborn has no material for, such as the tiles added after its release. Without it a facet holding such an id is refused. The Classic id itself is kept in the square.";
/// A land id of the Kingdom Reborn land map, where the choice starts.
const DEFAULT_KR_FALLBACK_LAND: u16 = 3;
const TARGET_CLIENT_HOVER: &str = "Its size table gives the size, its tiledata.mul checks the tile ids, and a client that draws facetNN.mul gets one in step with the map. It is only read.";
const FACET_IMAGES: [(FacetImageMode, &str); 4] = [
    (FacetImageMode::Update, "Update the target's image"),
    (FacetImageMode::Keep, "Keep the target's image"),
    (FacetImageMode::Regenerate, "Render the whole image"),
    (FacetImageMode::None, "No image"),
];

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SizeChoice {
    #[default]
    Automatic,
    Preset(usize),
    Custom,
}

pub(super) struct ConvertState {
    from_package: bool,
    /// A Classic folder other than the CC path.
    dir: Option<PathBuf>,
    package: Option<PathBuf>,
    facet: u8,
    edition: MapEdition,
    difs: DifChoice,
    target: FacetTarget,
    target_facet: u8,
    target_edition: MapEdition,
    size_choice: SizeChoice,
    custom_size: MapSizeCells,
    target_client: Option<PathBuf>,
    crop: bool,
    fill_land: u16,
    fill_z: i8,
    allow_lossy: bool,
    use_kr_fallback: bool,
    kr_fallback_land: u16,
    facet_image: FacetImageMode,
    worker: Option<mpsc::Receiver<Result<Vec<String>, String>>>,
    notes: Vec<String>,
    error: Option<String>,
}

impl Default for ConvertState {
    fn default() -> Self {
        let (_, custom_size) = SIZE_PRESETS[0];
        Self {
            from_package: false,
            dir: None,
            package: None,
            facet: 0,
            edition: MapEdition::Standard,
            difs: DifChoice::default(),
            target: FacetTarget::default(),
            target_facet: 0,
            target_edition: MapEdition::Standard,
            size_choice: SizeChoice::default(),
            custom_size,
            target_client: None,
            crop: false,
            fill_land: VOID_LAND_ID,
            fill_z: VOID_LAND_Z,
            allow_lossy: false,
            use_kr_fallback: false,
            kr_fallback_land: DEFAULT_KR_FALLBACK_LAND,
            facet_image: FacetImageMode::default(),
            worker: None,
            notes: Vec::new(),
            error: None,
        }
    }
}

impl ConvertState {
    /// The conversion to run into `output_dir`; `None` until it has a source.
    fn request(&self, classic_dir: &Path, output_dir: PathBuf) -> Option<ConvertRequest> {
        let source = if self.from_package {
            FacetSource::Package(self.package.clone()?)
        } else {
            let dir = self.dir.clone().unwrap_or_else(|| classic_dir.to_path_buf());
            FacetSource::Classic { dir, edition: self.edition, difs: self.difs }
        };
        let size = match self.size_choice {
            SizeChoice::Automatic => None,
            SizeChoice::Preset(index) => SIZE_PRESETS.get(index).map(|(_, size)| *size),
            SizeChoice::Custom => Some(self.custom_size),
        };
        Some(ConvertRequest {
            source,
            facet: self.facet,
            target: self.target,
            target_facet: self.target_facet,
            target_edition: self.target_edition,
            size,
            target_client: self.target_client.clone(),
            crop: self.crop,
            fill: LandFill { land: self.fill_land, z: self.fill_z },
            allow_lossy: self.allow_lossy,
            kr_fallback_land: self.use_kr_fallback.then_some(self.kr_fallback_land),
            facet_image: self.facet_image,
            output_dir,
        })
    }

    fn choose_package(&mut self, path: PathBuf) {
        if let Some((facet, edition)) = package_name_facet(&path) {
            self.facet = facet;
            self.target_facet = facet;
            self.edition = edition;
            self.target_edition = edition;
        }
        self.package = Some(path);
    }

    fn size_text(&self) -> &'static str {
        match self.size_choice {
            SizeChoice::Automatic => AUTOMATIC_SIZE_TEXT,
            SizeChoice::Preset(index) => SIZE_PRESETS.get(index).map_or(AUTOMATIC_SIZE_TEXT, |(name, _)| name),
            SizeChoice::Custom => CUSTOM_SIZE_TEXT,
        }
    }
}

fn poll(state: &mut ConvertState, ctx: &egui::Context) {
    let Some(worker) = &state.worker else {
        return;
    };
    match worker.try_recv() {
        Ok(Ok(notes)) => {
            state.notes = notes;
            state.error = None;
            state.worker = None;
        }
        Ok(Err(error)) => {
            state.error = Some(error);
            state.worker = None;
        }
        Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(POLL_INTERVAL),
        Err(mpsc::TryRecvError::Disconnected) => {
            state.error = Some("The conversion stopped without an answer: see the log.".to_owned());
            state.worker = None;
        }
    }
}

fn facet_combo(ui: &mut egui::Ui, id: &str, facet: &mut u8) {
    let name = |facet: u8| CLASSIC_FACET_NAMES.get(usize::from(facet)).copied().unwrap_or_default();
    egui::ComboBox::from_id_salt(id).selected_text(format!("{facet}: {}", name(*facet))).show_ui(ui, |ui| {
        for index in 0..CLASSIC_FACET_NAMES.len() as u8 {
            ui.selectable_value(facet, index, format!("{index}: {}", name(index)));
        }
    });
}

fn edition_combo(ui: &mut egui::Ui, id: &str, edition: &mut MapEdition) {
    egui::ComboBox::from_id_salt(id).selected_text(edition.to_string()).show_ui(ui, |ui| {
        for choice in MapEdition::ALL {
            ui.selectable_value(edition, choice, choice.to_string());
        }
    });
}

fn source_controls(state: &mut ConvertState, classic_dir: &Path, ui: &mut egui::Ui) {
    ui.strong("From");
    egui::ComboBox::from_id_salt("convert_source").selected_text(if state.from_package { "A facet package" } else { "A Classic folder" }).show_ui(ui, |ui| {
        ui.selectable_value(&mut state.from_package, false, "A Classic folder");
        ui.selectable_value(&mut state.from_package, true, "A facet package");
    });
    if state.from_package {
        if ui.button("Choose facetN.uop...").clicked() {
            let (filter, extensions) = PACKAGE_FILTER;
            if let Some(path) = crate::dialog::file_dialog().add_filter(filter, extensions).pick_file() {
                state.choose_package(path);
            }
        }
        match &state.package {
            Some(path) => ui.small(path.display().to_string()),
            None => ui.weak("No package chosen."),
        };
        return;
    }
    if ui.button("Other folder...").clicked() {
        if let Some(path) = crate::dialog::pick_folder() {
            state.dir = Some(path);
        }
    }
    ui.small(state.dir.as_deref().unwrap_or(classic_dir).display().to_string());
    let before = state.facet;
    facet_combo(ui, "convert_facet", &mut state.facet);
    if state.facet != before {
        state.target_facet = state.facet;
    }
    edition_combo(ui, "convert_edition", &mut state.edition);
    ui.horizontal(|ui| {
        ui.label("Difs");
        egui::ComboBox::from_id_salt("convert_difs").selected_text(state.difs.label()).show_ui(ui, |ui| {
            for choice in DifChoice::ALL {
                ui.selectable_value(&mut state.difs, choice, choice.label());
            }
        });
    });
}

fn target_controls(state: &mut ConvertState, ui: &mut egui::Ui) {
    ui.strong("To");
    egui::ComboBox::from_id_salt("convert_target").selected_text(state.target.label()).show_ui(ui, |ui| {
        for choice in FacetTarget::ALL {
            ui.selectable_value(&mut state.target, choice, choice.label());
        }
    });
    facet_combo(ui, "convert_target_facet", &mut state.target_facet);
    edition_combo(ui, "convert_target_edition", &mut state.target_edition);
    ui.horizontal(|ui| {
        if ui.button("Target client...").on_hover_text(TARGET_CLIENT_HOVER).clicked() {
            let (filter, extensions) = CLIENT_FILTER;
            if let Some(path) = crate::dialog::file_dialog().add_filter(filter, extensions).pick_file() {
                state.target_client = Some(path);
            }
        }
        if state.target_client.is_some() && ui.small_button("Clear").clicked() {
            state.target_client = None;
        }
    });
    if let Some(path) = &state.target_client {
        ui.small(path.display().to_string());
        if state.target.is_classic() {
            let shown = FACET_IMAGES.iter().find(|(mode, _)| *mode == state.facet_image).map_or("", |(_, name)| name);
            egui::ComboBox::from_id_salt("convert_facet_image").selected_text(shown).show_ui(ui, |ui| {
                for (mode, name) in FACET_IMAGES {
                    ui.selectable_value(&mut state.facet_image, mode, name);
                }
            });
        }
    }
    size_controls(state, ui);
    if state.target == FacetTarget::KingdomReborn {
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.use_kr_fallback, "Other land as").on_hover_text(KR_FALLBACK_HOVER);
            ui.add_enabled(state.use_kr_fallback, egui::DragValue::new(&mut state.kr_fallback_land).hexadecimal(LAND_HEX_DIGITS, false, true).prefix("0x"));
        });
    }
    if state.from_package {
        ui.checkbox(&mut state.allow_lossy, "Accept losses").on_hover_text("Convert a package even when the Classic files cannot hold all of it. The losses are listed.");
    }
}

fn size_controls(state: &mut ConvertState, ui: &mut egui::Ui) {
    ui.label("Size");
    egui::ComboBox::from_id_salt("convert_size").selected_text(state.size_text()).show_ui(ui, |ui| {
        ui.selectable_value(&mut state.size_choice, SizeChoice::Automatic, AUTOMATIC_SIZE_TEXT);
        for (index, (name, _)) in SIZE_PRESETS.iter().enumerate() {
            ui.selectable_value(&mut state.size_choice, SizeChoice::Preset(index), *name);
        }
        ui.selectable_value(&mut state.size_choice, SizeChoice::Custom, CUSTOM_SIZE_TEXT);
    });
    if state.size_choice == SizeChoice::Custom {
        ui.horizontal(|ui| {
            for side in [&mut state.custom_size.width, &mut state.custom_size.height] {
                ui.add(egui::DragValue::new(side).range(FACET_SIDE_ALIGNMENT..=MAX_FACET_SIDE).speed(FACET_SIDE_ALIGNMENT));
            }
        });
        ui.weak(format!("A side is a multiple of {FACET_SIDE_ALIGNMENT}."));
    }
    ui.checkbox(&mut state.crop, "Crop").on_hover_text("Lets a smaller size drop the land and statics outside it. The drop is listed. Refused otherwise.");
    ui.horizontal(|ui| {
        ui.label("Fill land").on_hover_text("What a larger size adds.");
        ui.add(egui::DragValue::new(&mut state.fill_land).hexadecimal(LAND_HEX_DIGITS, false, true).prefix("0x"));
        ui.label("z");
        ui.add(egui::DragValue::new(&mut state.fill_z));
    });
}

/// The section, below the controls of the map view.
pub(super) fn section(state: &mut ConvertState, classic_dir: &Path, ui: &mut egui::Ui, ctx: &egui::Context) {
    poll(state, ctx);
    egui::CollapsingHeader::new("Convert and save a facet").id_salt("classic_maps_convert").show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("classic_maps_convert_scroll").auto_shrink([false, true]).show(ui, |ui| {
            source_controls(state, classic_dir, ui);
            ui.separator();
            target_controls(state, ui);
            ui.separator();
            let busy = state.worker.is_some();
            let ready = !busy && (!state.from_package || state.package.is_some());
            let text = if busy { "Converting..." } else { "Save to a folder..." };
            if ui.add_enabled(ready, egui::Button::new(text)).clicked() {
                if let Some(output_dir) = crate::dialog::pick_folder() {
                    start(state, classic_dir, output_dir);
                }
            }
            if let Some(error) = &state.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            for note in &state.notes {
                ui.small(note);
            }
        });
    });
}

fn start(state: &mut ConvertState, classic_dir: &Path, output_dir: PathBuf) {
    let Some(request) = state.request(classic_dir, output_dir) else {
        return;
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(convert_facet(&request).map_err(|error| format!("{error:#}")));
    });
    state.worker = Some(receiver);
    state.notes.clear();
    state.error = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "/copy";

    #[test]
    fn a_size_preset_names_its_size() {
        for (name, size) in SIZE_PRESETS {
            assert!(name.ends_with(&format!(": {} x {}", size.width, size.height)), "{name}");
        }
    }
    const CLASSIC_DIR: &str = "/client";
    const PACKAGE: &str = "/client/facet2x.uop";

    #[test]
    fn a_package_source_needs_a_package_and_a_chosen_one_sets_the_facet() {
        let mut state = ConvertState { from_package: true, ..ConvertState::default() };
        assert!(state.request(Path::new(CLASSIC_DIR), PathBuf::from(OUTPUT)).is_none());
        state.choose_package(PathBuf::from(PACKAGE));
        let request = state.request(Path::new(CLASSIC_DIR), PathBuf::from(OUTPUT)).unwrap();
        assert!(matches!(request.source, FacetSource::Package(_)));
        assert_eq!((request.facet, request.target_facet, request.target_edition), (2, 2, MapEdition::Variant));
    }

    #[test]
    fn a_classic_source_defaults_to_the_cc_path_and_the_size_follows_the_choice() {
        let mut state = ConvertState::default();
        let request = state.request(Path::new(CLASSIC_DIR), PathBuf::from(OUTPUT)).unwrap();
        assert!(matches!(&request.source, FacetSource::Classic { dir, .. } if dir == Path::new(CLASSIC_DIR)));
        assert_eq!(request.size, None);
        state.size_choice = SizeChoice::Preset(0);
        assert_eq!(state.request(Path::new(CLASSIC_DIR), PathBuf::from(OUTPUT)).unwrap().size, Some(SIZE_PRESETS[0].1));
    }

    #[test]
    fn a_finished_conversion_shows_its_notes_and_a_lost_one_says_so() {
        let ctx = egui::Context::default();
        let mut state = ConvertState::default();
        let (sender, receiver) = mpsc::channel();
        state.worker = Some(receiver);
        sender.send(Ok(vec![OUTPUT.to_owned()])).unwrap();
        poll(&mut state, &ctx);
        assert_eq!(state.notes, vec![OUTPUT.to_owned()]);
        assert!(state.worker.is_none());

        let (sender, receiver) = mpsc::channel::<Result<Vec<String>, String>>();
        state.worker = Some(receiver);
        drop(sender);
        poll(&mut state, &ctx);
        assert!(state.error.is_some());
    }
}
