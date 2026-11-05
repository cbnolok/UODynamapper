//! The guide of the Effects tab: what each definition does in the client, and how the tab works.
//!
//! What it says about the client comes from
//! `scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md`, and the KDL grammar from
//! `uocf::enhanced::effect_definition`'s `kdl_text`.

use crate::logic::effects_editor::DefinitionKind;
use crate::ui::UOP_PATCHER_NOTE;
use eframe::egui;

const GUIDE_WINDOW_WIDTH: f32 = 620.0;
const GUIDE_WINDOW_HEIGHT: f32 = 600.0;

/// What the KDL text of effects is. Shown wherever effects are edited as text.
pub(crate) const EFFECT_KDL_HELP: &str = "\"EffectDefinitionCollection\" is the client file that gave this grammar its \
    name: EffectDefinitionCollection.uop. Animation frames store the same definitions, so both are edited with the \
    same text, in the KDL v1 language. Each node below is one definition. Children go in { }. What each one does \
    is in the Guide of the Effects tab.\n\n\
    particle <name index> [name=\"file\"] [offset-x= offset-y= camera-offset= scale= attach-mode= duration= looping= \
    field-3c= hued=]\n\
    sound <sound id> [value-08=]\n\
    round-robin { ... }\n\
    position-offset x0 y0 x1 y1 ... x7 y7\n\
    wait min=<ms> max=<ms> [aligned=] { ... }\n\
    lod { level <key> { ... } ... }\n\
    property-flag mode=\"flag-mask\"|\"property\" [mask=0x0] [property=0] { default { ... } case <key> { ... } }\n\
    earthquake amplitude= duration= frequency=\n\
    random { entry <weight> { ... } ... }\n\
    user-preference mask=<bits> { ... }\n\n\
    A particle's name= is informational. With a string dictionary it must match the index, or the index may be left \
    out and looked up by name. Fields left out take their defaults. Floats are written with a decimal point \
    (integers are accepted), and NaN and infinities as the strings \"nan\", \"inf\" and \"-inf\". Errors name the line and \
    column.\n\n\
    Example: wait min=500 max=2000 { particle 1234 name=\"flash.nif\" duration=0.5; sound 43 }";

/// One titled part of the guide.
struct Section {
    title: &'static str,
    text: &'static str,
}

const TAB_SECTIONS: &[Section] = &[
    Section {
        title: "What is listed on the left",
        text: "Collection: the entries of EffectDefinitionCollection.uop, played by id. Packets play id = packet value - 1 \
               (fixed effects, 0xC7 types 2 and 3, and explode effects); the packet paths of an entry are in its details, \
               and the lookup under the list finds the entry of a packet value.\n\n\
               TileArt: the effects stored in a tileart.uop record. They play on the items of that graphic, which gives \
               them an object: hue and Property flag work. They are read only here; the Tile Metadata tab edits them.\n\n\
               Files: the entries of EffectTexture.uop, which particles name: .nif scenes, .ems Chaos scripts and textures.",
    },
    Section {
        title: "How an effect is built",
        text: "An entry holds a tree of definitions, and its top-level definitions all play together. Particle, Sound and \
               Earthquake do something by themselves. Round robin, Wait, LOD, Property flag, Random and User preference \
               hold children and decide which of them play. So the tree reads as what plays, and under which condition.",
    },
    Section {
        title: "Editing",
        text: "Select a row on the left. The middle shows its fields and its tree of definitions; click a definition to edit \
               it under the tree. Hover a field's name to see what it does.\n\n\
               Add, on the Definitions line, adds a top-level definition. Add child to, on a selected definition, adds \
               inside it; a definition with several lists asks which. Keyed lists (LOD levels, Property flag cases, Random \
               entries) have their own add and remove buttons.\n\n\
               Ctrl+Z and Ctrl+Y undo and redo. Revert to loaded goes back to what was opened, and can be undone.",
    },
    Section {
        title: "Preview",
        text: "The right side plays the selection. Drag the picture to pan, right-drag to turn the camera, scroll to zoom and \
               double-click to reset the view. Drag the line under the picture to give it, or the lower part, more room.\n\n\
               Settings holds the playback and view options; they change the preview only, never the file. Parts says what \
               each definition of the selection does in the preview, and why one does not play. Export writes PNG frames, \
               a contact sheet or a glTF file.\n\n\
               The preview is this tool's simulation, not the client: the known differences are listed in \
               docs/agent_notes/effects_rendering_approximations.md.",
    },
    Section {
        title: "Validation",
        text: "The list at the bottom holds the problems found in the collection and in the Chaos scripts, such as a particle \
               naming a file that is not in EffectTexture.uop. Click a row to go to what it is about. It is rechecked after \
               each edit.",
    },
];

const SAVING_TEXT: &str = "Saving writes a copy: the loaded client files are refused as the target. Save Collection .uop keeps \
                           the source package and replaces exactly the edited entries, and EffectTexture .uop does the same \
                           for files. Export KDL writes the whole collection as text and Import KDL reads it back; hand-written \
                           text is read strictly, so a mistake is an error with its line and column.";

/// A definition's fields: the name in the editor, and what it does in the client.
const PARTICLE_FIELDS: &[(&str, &str)] = &[
    ("Name, Name index", "The .nif or .ems file in EffectTexture.uop, stored as an index into the string dictionary."),
    ("Offset x", "Tiles east of the effect's position."),
    ("Offset y", "Tiles south of the effect's position."),
    ("Camera offset", "Along the camera's up direction: a positive value moves the effect up on screen."),
    ("Scale", "The scale of the scene; the client uses its absolute value."),
    (
        "Attach mode",
        "2: the particle is not attached to an item on the ground, and plays only while the item is shown in flight (thrown \
         models). Client data has 2 on thrown models, 0 on arrows and 1 elsewhere; no difference between 0 and 1 was found.",
    ),
    (
        "Duration (s)",
        "A non-looping effect stops once this much time has passed; a looping one wraps its time when this is not 0. It is \
         independent of a packet's duration, which counts update ticks and applies only to single-definition entries.",
    ),
    ("Looping", "Restarts the effect when it ends."),
    ("field_3c", "Must be 0: the client refuses the effect otherwise. It is 0 in all client data."),
    ("Hued", "Takes the hue of the object it plays on, when that hue is not 0. Not for Chaos particles."),
];
const SOUND_FIELDS: &[(&str, &str)] = &[
    ("Sound id", "The sound played at the effect's position, with distance attenuation."),
    ("value_08", "Copied but never read by the client; 1.0 in all client data."),
];
const ROUND_ROBIN_FIELDS: &[(&str, &str)] = &[(
    "Children",
    "One plays per use, in order. The position in the cycle belongs to the definition, so every object of the same graphic \
     and every play of the entry share it.",
)];
const POSITION_OFFSET_FIELDS: &[(&str, &str)] = &[(
    "Direction 0 to 7",
    "An x, y offset per direction. The effect object itself never reads them; the client reads the definition from a mount's \
     animation sequence, to place the rider. No shipped collection, tileart or sequence data uses it.",
)];
const WAIT_FIELDS: &[(&str, &str)] = &[
    ("Min delay, Max delay (ms)", "After firing, the next fire is at least min and at most max milliseconds away, chosen at random."),
    ("Aligned", "The next fire starts from the next multiple of the min delay, not from now."),
    (
        "Children",
        "Played only when the definition is used and the next fire time has passed. It is a throttle, not a timer: it never \
         fires by itself. The first use always fires.",
    ),
];
const LOD_FIELDS: &[(&str, &str)] = &[(
    "Levels",
    "Children per key. The client plays the key equal to the particle detail setting (Low, Medium, High are 1, 2, 3; the \
     default is 3), else the next lower key down to 0, else nothing.",
)];
const PROPERTY_FLAG_FIELDS: &[(&str, &str)] = &[
    ("Mode", "Flag mask: the key is whether (object flags & mask) is not 0. Property: the key is the object value named by Property."),
    ("Mask", "The object flags tested in flag mask mode."),
    ("Property", "The object value in property mode: 7 is the object's hue; 2, 3 and 8 are other object values; anything else is that tileart property."),
    ("Default children", "Property mode: played when no case matches. In flag mask mode no match plays nothing."),
    ("Cases", "Children per key."),
    (
        "No object",
        "It needs an object. Collection entries played from packets have none, so there only the default children of property \
         mode can play.",
    ),
];
const EARTHQUAKE_FIELDS: &[(&str, &str)] = &[
    ("Amplitude", "The camera target's x moves by amplitude * sin(time * frequency * 2 pi)."),
    ("Duration (s)", "The effect ends when this much time has passed."),
    ("Frequency (Hz)", "The speed of the shake."),
];
const RANDOM_FIELDS: &[(&str, &str)] = &[(
    "Entries",
    "Weight and children. The weights are percentages, and one entry is drawn per use. If they add up to less than 100, the \
     rest of the time nothing plays; anything beyond 100 is never reached.",
)];
const USER_PREFERENCE_FIELDS: &[(&str, &str)] = &[
    (
        "Mask",
        "The children are skipped when all these bits are set in the user's effect setting. Bit 0 means footsteps are disabled, \
         and the client writes no other bit. A mask of 1 also skips them when an object value the client tests (vt+0xa0, not \
         named) is negative.",
    ),
    ("Children", "The definitions this switches."),
];

fn kind_fields(kind: DefinitionKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        DefinitionKind::Particle => PARTICLE_FIELDS,
        DefinitionKind::Sound => SOUND_FIELDS,
        DefinitionKind::RoundRobin => ROUND_ROBIN_FIELDS,
        DefinitionKind::PositionOffset => POSITION_OFFSET_FIELDS,
        DefinitionKind::Wait => WAIT_FIELDS,
        DefinitionKind::Lod => LOD_FIELDS,
        DefinitionKind::PropertyFlag => PROPERTY_FLAG_FIELDS,
        DefinitionKind::Earthquake => EARTHQUAKE_FIELDS,
        DefinitionKind::Random => RANDOM_FIELDS,
        DefinitionKind::UserPreference => USER_PREFERENCE_FIELDS,
    }
}

fn kind_section(ui: &mut egui::Ui, kind: DefinitionKind) {
    egui::CollapsingHeader::new(kind.label()).id_salt(("effects_guide_kind", kind.label())).show(ui, |ui| {
        ui.label(kind.description());
        ui.add_space(ui.spacing().item_spacing.y);
        egui::Grid::new(("effects_guide_fields", kind.label())).num_columns(2).striped(true).show(ui, |ui| {
            for (field, meaning) in kind_fields(kind) {
                ui.strong(*field);
                ui.add(egui::Label::new(*meaning).wrap());
                ui.end_row();
            }
        });
    });
}

/// The guide, while `open` is set. It does not block the tab, so it can stay open during editing.
pub(super) fn guide_window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("Effects guide")
        .open(open)
        .default_size(egui::vec2(GUIDE_WINDOW_WIDTH, GUIDE_WINDOW_HEIGHT))
        .vscroll(true)
        .show(ctx, |ui| {
            ui.heading("How this tab works");
            for section in TAB_SECTIONS {
                egui::CollapsingHeader::new(section.title).default_open(true).show(ui, |ui| ui.label(section.text));
            }
            egui::CollapsingHeader::new("Saving").default_open(true).show(ui, |ui| {
                ui.label(SAVING_TEXT);
                ui.label(UOP_PATCHER_NOTE);
            });
            egui::CollapsingHeader::new("KDL text").show(ui, |ui| ui.label(EFFECT_KDL_HELP));
            ui.separator();
            ui.heading("What each definition does");
            for kind in DefinitionKind::ALL {
                kind_section(ui, kind);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_definition_kind_has_its_fields_explained() {
        for kind in DefinitionKind::ALL {
            assert!(!kind_fields(kind).is_empty(), "{}", kind.label());
        }
    }

    #[test]
    fn the_guide_draws_without_panicking() {
        let ctx = egui::Context::default();
        let mut open = true;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| guide_window(ui.ctx(), &mut open));
        assert!(open);
    }
}
