//! The paperdoll windows' right-click menu: what a robe takes off, garment by garment. A right-click picks the item
//! drawn on top under the pointer, by the alpha of each drawn item's texels. While a dress-layer item is drawn, the
//! menu offers "Show under this robe", "Hide under this robe" and "Back to measured" for a garment drawn beneath it,
//! and lists the garments taken off beneath it, each to be brought back. A choice goes to the user's copy of
//! `PaperdollOverrides.kdl` (`cache/runtime_specs/paperdoll/`, `configs::runtime_specs`), never to the bundled file:
//! `udd_assets::paperdoll_overrides::set_garment_rule` edits it in place, keeping its comments and other covers, for
//! the covering item on the body frame drawn, and the paperdolls are drawn again. Both the 3D and the gump paperdolls
//! take the menu; egui draws it as a popup of the window, no viewport of its own.

use bevy_egui::egui;
use color_eyre::eyre;
use udd_assets::paperdoll_overrides::{
    set_garment_rule, GarmentChoice, GarmentRuleEdit, OverrideRule, PaperdollOverrides, DRESS_SLOT,
    PAPERDOLL_OVERRIDES_FILE_NAME,
};
use uocf::enhanced::paperdoll::slot_layer;

use super::paperdoll_overrides::TakenOff;
use crate::configs::runtime_specs::{read_text, SpecFiles};

/// A texel at least this opaque picks its item; fainter ones (antialiased edges, shadows) let the item beneath it be
/// picked, and only where no item is that opaque does the faintest texel drawn pick its item.
const PICKED_ALPHA_MIN: u8 = 128;
const RGBA_BYTES: usize = 4;
const ALPHA_CHANNEL: usize = 3;

const SHOW_LABEL: &str = "Show under this robe";
const HIDE_LABEL: &str = "Hide under this robe";
const MEASURED_LABEL: &str = "Back to measured";
const SHOW_HOVER: &str = "Keep this garment beneath this robe on this body, whatever the measurement says.";
const HIDE_HOVER: &str = "Take this garment off beneath this robe on this body, whatever the measurement says.";
const MEASURED_HOVER: &str =
    "No rule of yours for this garment: your slot rules, else what paperdoll_ec.uddp measured, decide.";
const NO_BODY_FRAME_TEXT: &str = "The choices need paperdoll_ec.uddp: they apply to its body frame.";
const CLOSE_LABEL: &str = "Close window";

/// Writes `edit` into the user's copy of `files`, started from the bundled file when there is none, through a
/// temporary file renamed over it, and returns the overrides it now holds. A copy the overrides do not read is refused
/// (`set_garment_rule`) and stays as it is, so the menu writes nothing to it until it is fixed.
pub fn write_choice(files: &SpecFiles, edit: &GarmentRuleEdit) -> eyre::Result<PaperdollOverrides> {
    let user = &files.user;
    let text = read_text(if user.exists() { user } else { &files.bundled })?;
    let source_name = user.display().to_string();
    let edited = set_garment_rule(&source_name, &text, edit)?;
    let overrides = PaperdollOverrides::decode(&source_name, &edited)?;
    if let Some(parent) = user.parent() {
        std::fs::create_dir_all(parent).map_err(|error| eyre::eyre!("{}: cannot create: {error}", parent.display()))?;
    }
    let temporary = user.with_file_name(format!("{PAPERDOLL_OVERRIDES_FILE_NAME}.tmp"));
    std::fs::write(&temporary, edited).map_err(|error| eyre::eyre!("{}: cannot write: {error}", temporary.display()))?;
    std::fs::rename(&temporary, user).map_err(|error| eyre::eyre!("{}: cannot write: {error}", user.display()))?;
    Ok(overrides)
}

/// One drawn item's texels on a paperdoll image, for picking: the alpha over its rect, in image texels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickLayer {
    /// The item's layer and id.
    pub item: (usize, u32),
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Each texel's alpha, rows top first.
    pub alpha: Vec<u8>,
}

impl PickLayer {
    /// The layer of an item drawn as `rgba` (`width` x `height` texels) at `(x, y)` on the image.
    pub fn from_rgba(item: (usize, u32), x: i32, y: i32, width: u32, height: u32, rgba: &[u8]) -> Self {
        let alpha = rgba.chunks_exact(RGBA_BYTES).map(|texel| texel[ALPHA_CHANNEL]).collect();
        Self { item, x, y, width, height, alpha }
    }

    fn alpha_at(&self, x: i32, y: i32) -> u8 {
        let (column, row) = (x - self.x, y - self.y);
        if column < 0 || row < 0 || column >= self.width as i32 || row >= self.height as i32 {
            return 0;
        }
        self.alpha.get(row as usize * self.width as usize + column as usize).copied().unwrap_or(0)
    }
}

/// What a paperdoll window draws, for its menu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaperdollDrawn {
    /// The image's size in texels.
    pub width: u32,
    pub height: u32,
    /// The texels of each item drawn, bottom first.
    pub layers: Vec<PickLayer>,
    /// The items drawn, bottom first (each its layer and id), with or without texels.
    pub drawn: Vec<(usize, u32)>,
    /// What was taken off beneath a covering item.
    pub taken: Vec<TakenOff>,
    /// The number of the body frame the stored decisions and the overrides' `bodies=` are looked up on; `None`
    /// without paperdoll_ec.uddp.
    pub body_frame: Option<u32>,
}

/// An item in the menu: its layer and id, and how the menu names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub item: (usize, u32),
    pub name: Option<String>,
}

impl MenuItem {
    fn new(item: (usize, u32), name: &impl Fn(u32) -> Option<String>) -> Self {
        Self { item, name: name(item.1) }
    }

    /// `name (0x1F03)`, or the id alone.
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{name} (0x{:04X})", self.item.1),
            None => format!("0x{:04X}", self.item.1),
        }
    }
}

/// The menu a right-click opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperdollMenu {
    /// The item drawn on top under the pointer.
    pub picked: Option<MenuItem>,
    /// The dress-layer item drawn: the covering item the choices are for.
    pub covering: Option<MenuItem>,
    /// The picked item, when it is drawn beneath the covering item.
    pub garment: Option<MenuItem>,
    /// The garments taken off beneath the covering item.
    pub hidden: Vec<MenuItem>,
    pub body_frame: Option<u32>,
}

/// A choice made in the menu, kept until it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenRule {
    pub covering: MenuItem,
    pub garment: MenuItem,
    pub body: u32,
    pub choice: GarmentChoice,
}

impl ChosenRule {
    pub fn edit(&self) -> GarmentRuleEdit<'_> {
        GarmentRuleEdit {
            covering: self.covering.item,
            covering_name: self.covering.name.as_deref(),
            body: self.body,
            garment: self.garment.item,
            garment_name: self.garment.name.as_deref(),
            choice: self.choice,
        }
    }

    /// The choice in a message.
    pub fn describe(&self) -> String {
        let (garment, covering, body) = (self.garment.label(), self.covering.label(), self.body);
        match self.choice {
            GarmentChoice::Show => format!("{garment} is shown under {covering} on body {body}"),
            GarmentChoice::Hide => format!("{garment} is hidden under {covering} on body {body}"),
            GarmentChoice::Measured => format!("{garment} under {covering} on body {body} is back to measured"),
        }
    }
}

/// What the user picked in the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Choose(ChosenRule),
    CloseWindow,
}

impl PaperdollDrawn {
    /// The item drawn on top at `(u, v)`, the fraction of the image across and down: the topmost with a texel at least
    /// [`PICKED_ALPHA_MIN`] opaque there, else the topmost drawn there at all.
    pub fn item_at(&self, u: f32, v: f32) -> Option<(usize, u32)> {
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return None;
        }
        let x = (u * self.width as f32) as i32;
        let y = (v * self.height as f32) as i32;
        let topmost =
            |min: u8| self.layers.iter().rev().find(|layer| layer.alpha_at(x, y) >= min).map(|layer| layer.item);
        topmost(PICKED_ALPHA_MIN).or_else(|| topmost(1))
    }

    /// The menu of a right-click at `(u, v)`; `name` names an item.
    pub fn menu_at(&self, u: f32, v: f32, name: impl Fn(u32) -> Option<String>) -> PaperdollMenu {
        let dress = slot_layer(DRESS_SLOT).ok();
        let picked = self.item_at(u, v);
        let covering_index = self.drawn.iter().position(|&(layer, _)| Some(layer) == dress);
        let covering = covering_index.map(|index| self.drawn[index]);
        let beneath = |item: (usize, u32)| {
            covering_index.is_some_and(|covering| self.drawn[..covering].contains(&item))
        };
        let hidden = self
            .taken
            .iter()
            .filter(|taken| Some(taken.covering) == covering)
            .map(|taken| MenuItem::new(taken.garment, &name))
            .collect();
        PaperdollMenu {
            picked: picked.map(|item| MenuItem::new(item, &name)),
            covering: covering.map(|item| MenuItem::new(item, &name)),
            garment: picked.filter(|&item| beneath(item)).map(|item| MenuItem::new(item, &name)),
            hidden,
            body_frame: self.body_frame,
        }
    }
}

/// What the overrides decide for `garment` beneath `covering` on `body`, as the menu shows it.
pub fn current_choice(
    overrides: &PaperdollOverrides,
    covering: (usize, u32),
    body: u32,
    garment: (usize, u32),
) -> GarmentChoice {
    match overrides.decide(covering, Some(body), garment) {
        Some(decision) if decision.rule == OverrideRule::Garment && decision.takes_off => GarmentChoice::Hide,
        Some(decision) if decision.rule == OverrideRule::Garment => GarmentChoice::Show,
        _ => GarmentChoice::Measured,
    }
}

/// The three choices for `garment` beneath the covering item, the current one checked; the one clicked, if another.
fn choice_buttons(
    ui: &mut egui::Ui,
    menu: &PaperdollMenu,
    overrides: &PaperdollOverrides,
    garment: &MenuItem,
) -> Option<MenuAction> {
    let (Some(covering), Some(body)) = (&menu.covering, menu.body_frame) else {
        ui.label(NO_BODY_FRAME_TEXT);
        return None;
    };
    let current = current_choice(overrides, covering.item, body, garment.item);
    let choices = [
        (GarmentChoice::Show, SHOW_LABEL, SHOW_HOVER),
        (GarmentChoice::Hide, HIDE_LABEL, HIDE_HOVER),
        (GarmentChoice::Measured, MEASURED_LABEL, MEASURED_HOVER),
    ];
    let mut chosen = None;
    for (choice, label, hover) in choices {
        if ui.selectable_label(current == choice, label).on_hover_text(hover).clicked() && current != choice {
            chosen = Some(MenuAction::Choose(ChosenRule {
                covering: covering.clone(),
                garment: garment.clone(),
                body,
                choice,
            }));
        }
    }
    chosen
}

/// Draws the menu, `notice` first if any; returns what was picked, if anything, and closes the menu then.
pub fn menu_ui(
    ui: &mut egui::Ui,
    menu: &PaperdollMenu,
    overrides: &PaperdollOverrides,
    notice: Option<&str>,
) -> Option<MenuAction> {
    let mut action = None;
    if let Some(notice) = notice {
        ui.colored_label(ui.visuals().warn_fg_color, notice);
        ui.separator();
    }
    match &menu.covering {
        None => {
            if let Some(picked) = &menu.picked {
                ui.label(picked.label());
            }
            ui.weak("No robe or dress is drawn.");
        }
        Some(covering) => {
            match (&menu.garment, &menu.picked) {
                (Some(garment), _) => {
                    ui.label(format!("{} under {}", garment.label(), covering.label()));
                    action = choice_buttons(ui, menu, overrides, garment);
                }
                (None, Some(picked)) => {
                    ui.label(picked.label());
                }
                (None, None) => {}
            }
            ui.separator();
            if menu.hidden.is_empty() {
                ui.weak(format!("Nothing is hidden under {}.", covering.label()));
            } else {
                ui.label(format!("Hidden under {}:", covering.label()));
                for hidden in &menu.hidden {
                    ui.menu_button(hidden.label(), |ui| {
                        if let Some(chosen) = choice_buttons(ui, menu, overrides, hidden) {
                            action = Some(chosen);
                        }
                    });
                }
            }
        }
    }
    ui.separator();
    if ui.button(CLOSE_LABEL).clicked() {
        action = Some(MenuAction::CloseWindow);
    }
    if action.is_some() {
        ui.close();
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIRT: u32 = 0x1517;
    const PLATE_LEGS: u32 = 0x1411;
    const ROBE: u32 = 0x1F03;
    const SASH: u32 = 5441;
    const BODY: u32 = 400;
    const OPAQUE: u8 = 255;
    const FAINT: u8 = 40;

    fn slot(name: &str) -> usize {
        slot_layer(name).unwrap()
    }

    fn item(name: &str, item_id: u32) -> (usize, u32) {
        (slot(name), item_id)
    }

    /// A `width` x 1 layer at `x` with each texel's alpha.
    fn layer(item: (usize, u32), x: i32, alpha: &[u8]) -> PickLayer {
        PickLayer { item, x, y: 0, width: alpha.len() as u32, height: 1, alpha: alpha.to_vec() }
    }

    fn name(item_id: u32) -> Option<String> {
        (item_id == ROBE).then(|| "robe".to_string())
    }

    /// Plate legs on texels 0 to 3, a robe over texels 1 to 3, faint at 3, a sash over texel 2 alone, and the shirt
    /// taken off beneath the robe; a 4 x 1 image.
    fn drawn() -> PaperdollDrawn {
        let (legs, robe, sash) = (item("legs", PLATE_LEGS), item("dress", ROBE), item("abovechest", SASH));
        PaperdollDrawn {
            width: 4,
            height: 1,
            layers: vec![
                layer(legs, 0, &[OPAQUE; 4]),
                layer(robe, 1, &[OPAQUE, OPAQUE, FAINT]),
                layer(sash, 2, &[OPAQUE]),
            ],
            drawn: vec![legs, robe, sash],
            taken: vec![TakenOff { garment: item("torso", SHIRT), covering: robe }],
            body_frame: Some(BODY),
        }
    }

    /// `u` at the middle of texel `x` of the 4 texels.
    fn at(x: u32) -> f32 {
        const TEXELS: f32 = 4.0;
        (x as f32 + 0.5) / TEXELS
    }

    /// The topmost item at least half opaque under the pointer is picked; a faint texel lets the one beneath show
    /// through; nothing off the image.
    #[test]
    fn the_topmost_opaque_item_is_picked() {
        let drawn = drawn();
        assert_eq!(drawn.item_at(at(0), 0.5), Some(item("legs", PLATE_LEGS)));
        assert_eq!(drawn.item_at(at(1), 0.5), Some(item("dress", ROBE)));
        assert_eq!(drawn.item_at(at(2), 0.5), Some(item("abovechest", SASH)));
        assert_eq!(drawn.item_at(at(3), 0.5), Some(item("legs", PLATE_LEGS)), "a faint robe texel");
        let faint_only = PaperdollDrawn { layers: vec![layer(item("dress", ROBE), 0, &[FAINT; 4])], ..drawn.clone() };
        assert_eq!(faint_only.item_at(at(0), 0.5), Some(item("dress", ROBE)));
        assert_eq!(drawn.item_at(1.0, 0.5), None);
        assert_eq!(drawn.item_at(at(0), -0.1), None);
    }

    /// A garment picked beneath the robe gets the choices; the robe or an item over it lists what the robe hides.
    #[test]
    fn the_menu_offers_the_garments_beneath_the_robe() {
        let drawn = drawn();
        let robe = MenuItem { item: item("dress", ROBE), name: Some("robe".to_string()) };
        let shirt = MenuItem { item: item("torso", SHIRT), name: None };
        let legs = drawn.menu_at(at(0), 0.5, name);
        assert_eq!(legs.covering, Some(robe.clone()));
        assert_eq!(legs.garment.map(|garment| garment.item), Some(item("legs", PLATE_LEGS)));
        assert_eq!(legs.hidden, [shirt.clone()]);
        let over = drawn.menu_at(at(2), 0.5, name);
        assert_eq!((over.garment, over.picked.map(|picked| picked.item)), (None, Some(item("abovechest", SASH))));
        assert_eq!(robe.label(), "robe (0x1F03)");
        let bare = PaperdollDrawn { drawn: vec![item("legs", PLATE_LEGS)], taken: Vec::new(), ..drawn };
        let menu = bare.menu_at(at(0), 0.5, name);
        assert_eq!((menu.covering, menu.garment, menu.hidden), (None, None, Vec::new()));
    }

    /// The choice shown checked is what the overrides decide for the garment: a garment rule, else back to measured.
    #[test]
    fn the_current_choice_follows_the_garment_rules() {
        let overrides = PaperdollOverrides::decode(
            PAPERDOLL_OVERRIDES_FILE_NAME,
            "cover \"dress\" items=\"0x1F03\" bodies=\"400\" {\n    hide \"legs\"\n    show items=\"0x1517\"\n}\n",
        )
        .unwrap();
        let robe = item("dress", ROBE);
        assert_eq!(current_choice(&overrides, robe, BODY, item("torso", SHIRT)), GarmentChoice::Show);
        assert_eq!(current_choice(&overrides, robe, BODY, item("legs", PLATE_LEGS)), GarmentChoice::Measured);
        assert_eq!(current_choice(&overrides, robe, BODY + 1, item("torso", SHIRT)), GarmentChoice::Measured);
    }

    /// A choice goes to the user's copy, started from the bundled file, which stays as it is; the next choice edits
    /// the copy. An invalid copy is neither read nor written.
    #[test]
    fn choices_go_to_the_users_copy() {
        static NEXT_DIRECTORY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        const BUNDLED: &str = "// The bundled file.\n";
        const SPECS_FOLDER: &str = "paperdoll";
        let ordering = std::sync::atomic::Ordering::Relaxed;
        let root = std::env::temp_dir().join(format!(
            "dynamapper-paperdoll-menu-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, ordering)
        ));
        let files =
            SpecFiles::under(&root.join("assets"), &root.join("cache"), SPECS_FOLDER, PAPERDOLL_OVERRIDES_FILE_NAME);
        let (bundled, user) = (files.bundled.clone(), files.user.clone());
        let load = || files.load(PaperdollOverrides::decode).unwrap();
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, BUNDLED).unwrap();
        assert_eq!(load().path, bundled);

        let chosen = |garment: u32, choice| ChosenRule {
            covering: MenuItem { item: item("dress", ROBE), name: Some("robe".to_string()) },
            garment: MenuItem { item: item("torso", garment), name: None },
            body: BODY,
            choice,
        };
        let shown = write_choice(&files, &chosen(SHIRT, GarmentChoice::Show).edit()).unwrap();
        assert_eq!(current_choice(&shown, item("dress", ROBE), BODY, item("torso", SHIRT)), GarmentChoice::Show);
        assert_eq!(std::fs::read_to_string(&bundled).unwrap(), BUNDLED);
        let written = std::fs::read_to_string(&user).unwrap();
        assert!(written.starts_with(BUNDLED), "{written}");
        let loaded = load();
        assert_eq!((loaded.path, loaded.value, loaded.user_error.is_none()), (user.clone(), shown, true));

        write_choice(&files, &chosen(0x1F7B, GarmentChoice::Hide).edit()).unwrap();
        let both = load().value;
        assert_eq!(current_choice(&both, item("dress", ROBE), BODY, item("torso", 0x1F7B)), GarmentChoice::Hide);
        assert_eq!(current_choice(&both, item("dress", ROBE), BODY, item("torso", SHIRT)), GarmentChoice::Show);

        std::fs::write(&user, "cover \"robe\" items=\"any\"\n").unwrap();
        let invalid = load();
        assert_eq!((invalid.path, invalid.user_error.is_some()), (bundled.clone(), true));
        assert!(write_choice(&files, &chosen(SHIRT, GarmentChoice::Hide).edit()).is_err());
        assert_eq!(std::fs::read_to_string(&user).unwrap(), "cover \"robe\" items=\"any\"\n");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
