//! Checks of effect definitions against what the client does with them
//! (`scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md`, "Runtime semantics").
//!
//! Unknown definition type keys need no check: they cannot be represented ([`EffectDefinition`] has one variant
//! per client type) and both readers (binary and KDL) reject them.

use std::fmt;

use super::collection::{EffectDefinitionCollection, EffectEntryKey};
use super::{
    EffectDefinition, EffectMapEntry, PropertyFlagMode, MAX_EFFECT_NESTING_DEPTH, PARTICLE_ATTACH_PROJECTILE_ONLY,
    PARTICLE_LOD_MAX, RANDOM_WEIGHT_TOTAL, USER_PREFERENCE_FOOTSTEPS,
};
use crate::enhanced::string_dictionary::UoStringDictionary;

/// Difference from [`RANDOM_WEIGHT_TOTAL`] below which a weight sum counts as the total (float rounding of
/// weights like 33.3).
pub const RANDOM_WEIGHT_TOLERANCE: f32 = 0.01;
/// The `value_08` of every client sound definition; the client never reads it.
const SOUND_VALUE_08_CLIENT: f32 = 1.0;
/// Keys a property flag map can match in [`PropertyFlagMode::FlagMask`] mode: `(flags & mask) != 0`.
const FLAG_MASK_KEYS: [u32; 2] = [0, 1];

/// What the validation can look up.
#[derive(Default, Clone, Copy)]
pub struct EffectValidationContext<'a> {
    /// The client string dictionary, to resolve particle names. `None` skips the name checks.
    pub strings: Option<&'a UoStringDictionary>,
    /// Whether the particle file of a name exists (`EffectTexturePackage::contains`). Called with the name as
    /// the string dictionary holds it. `None` (or no `strings`) skips the check.
    pub particle_file_exists: Option<&'a dyn Fn(&str) -> bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectIssueSeverity {
    /// The client plays it, but likely not as intended.
    Warning,
    /// The client fails to play it, or the readers reject it.
    Error,
}

/// One validation finding.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectIssue {
    /// The collection entry; `None` when validating a bare definition list.
    pub entry: Option<EffectEntryKey>,
    /// Where the definition is: top-level index, then `/`-separated nested positions
    /// (`2/entry 1/0` = first child of the second random entry of the third definition).
    pub path: String,
    pub kind: EffectIssueKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EffectIssueKind {
    /// A collection entry without definitions plays nothing.
    EmptyEntry,
    /// Deeper than [`MAX_EFFECT_NESTING_DEPTH`]: the readers reject it.
    NestingTooDeep,
    /// The particle name index is not in the string dictionary.
    UnresolvedParticleName { name_index: u32 },
    /// The particle file is not in `EffectTexture.uop`: the client plays nothing for it.
    MissingParticleFile { name: String },
    /// `field_3c` must be 0: the client refuses to start the particle otherwise.
    NonZeroField3c { value: u8 },
    /// An attach mode other than 0, 1 and [`PARTICLE_ATTACH_PROJECTILE_ONLY`] (the client treats it as 1).
    UnknownAttachMode { value: u8 },
    /// A NaN or infinite float.
    NonFiniteValue { field: &'static str },
    /// The client never reads the sound `value_08` (1.0 in all client data).
    UnusedSoundValue { value: f32 },
    /// The weights do not sum to [`RANDOM_WEIGHT_TOTAL`]: below, the remainder plays nothing; above, the excess
    /// is unreachable.
    RandomWeightSum { sum: f32 },
    /// A negative random weight: its entry never plays.
    NegativeRandomWeight { weight: f32 },
    /// `min_delay_ms` > `max_delay_ms`: the client's random range `max - min` wraps to a huge delay.
    WaitMinAboveMax { min: u32, max: u32 },
    /// A LOD level above [`PARTICLE_LOD_MAX`] is never selected.
    LodLevelUnreachable { key: u32 },
    /// A repeated map key: the client keeps the last list.
    DuplicateMapKey { key: u32 },
    /// A flag-mask case key other than 0 and 1 is never matched.
    FlagMaskCaseUnreachable { key: u32 },
    /// Default children of a flag-mask property flag effect never play.
    FlagMaskDefaultUnused,
    /// Position offsets are inert in this client build.
    InertPositionOffset,
    /// A user preference mask of 0 skips its children always.
    UserPreferenceZeroMask,
    /// Mask bits other than [`USER_PREFERENCE_FOOTSTEPS`] are never set by the client: the children always play.
    UserPreferenceUnknownBits { mask: u32 },
}

impl EffectIssue {
    pub fn severity(&self) -> EffectIssueSeverity {
        match self.kind {
            EffectIssueKind::NestingTooDeep
            | EffectIssueKind::UnresolvedParticleName { .. }
            | EffectIssueKind::MissingParticleFile { .. }
            | EffectIssueKind::NonZeroField3c { .. }
            | EffectIssueKind::NonFiniteValue { .. }
            | EffectIssueKind::WaitMinAboveMax { .. } => EffectIssueSeverity::Error,
            EffectIssueKind::EmptyEntry
            | EffectIssueKind::UnknownAttachMode { .. }
            | EffectIssueKind::UnusedSoundValue { .. }
            | EffectIssueKind::RandomWeightSum { .. }
            | EffectIssueKind::NegativeRandomWeight { .. }
            | EffectIssueKind::LodLevelUnreachable { .. }
            | EffectIssueKind::DuplicateMapKey { .. }
            | EffectIssueKind::FlagMaskCaseUnreachable { .. }
            | EffectIssueKind::FlagMaskDefaultUnused
            | EffectIssueKind::InertPositionOffset
            | EffectIssueKind::UserPreferenceZeroMask
            | EffectIssueKind::UserPreferenceUnknownBits { .. } => EffectIssueSeverity::Warning,
        }
    }
}

impl fmt::Display for EffectIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.entry {
            Some(EffectEntryKey::Id(id)) => write!(f, "entry {id}")?,
            Some(EffectEntryKey::Hash(hash)) => write!(f, "entry {hash:016X}")?,
            None => f.write_str("definitions")?,
        }
        if !self.path.is_empty() {
            write!(f, " [{}]", self.path)?;
        }
        f.write_str(": ")?;
        match &self.kind {
            EffectIssueKind::EmptyEntry => f.write_str("no definitions: plays nothing"),
            EffectIssueKind::NestingTooDeep => {
                write!(f, "nested deeper than {MAX_EFFECT_NESTING_DEPTH} levels")
            }
            EffectIssueKind::UnresolvedParticleName { name_index } => {
                write!(f, "particle name index {name_index} is not in the string dictionary")
            }
            EffectIssueKind::MissingParticleFile { name } => write!(f, "particle file \"{name}\" not found"),
            EffectIssueKind::NonZeroField3c { value } => {
                write!(f, "field_3c is {value}: the client refuses to start the particle unless it is 0")
            }
            EffectIssueKind::UnknownAttachMode { value } => write!(f, "unknown attach mode {value}"),
            EffectIssueKind::NonFiniteValue { field } => write!(f, "{field} is NaN or infinite"),
            EffectIssueKind::UnusedSoundValue { value } => {
                write!(f, "sound value_08 {value} has no effect (the client never reads it)")
            }
            EffectIssueKind::RandomWeightSum { sum } if *sum < RANDOM_WEIGHT_TOTAL => {
                write!(f, "random weights sum to {sum}: {}% chance of nothing", RANDOM_WEIGHT_TOTAL - sum)
            }
            EffectIssueKind::RandomWeightSum { sum } => {
                write!(f, "random weights sum to {sum}: the excess over {RANDOM_WEIGHT_TOTAL} is unreachable")
            }
            EffectIssueKind::NegativeRandomWeight { weight } => {
                write!(f, "random weight {weight} is negative: its entry never plays")
            }
            EffectIssueKind::WaitMinAboveMax { min, max } => {
                write!(f, "wait min delay {min} ms is above the max {max} ms")
            }
            EffectIssueKind::LodLevelUnreachable { key } => {
                write!(f, "LOD level {key} is above {PARTICLE_LOD_MAX}: never selected")
            }
            EffectIssueKind::DuplicateMapKey { key } => write!(f, "key {key} repeats: the client keeps the last"),
            EffectIssueKind::FlagMaskCaseUnreachable { key } => {
                write!(f, "flag-mask case {key} never matches (the key is 0 or 1)")
            }
            EffectIssueKind::FlagMaskDefaultUnused => {
                f.write_str("default children of a flag-mask property flag effect never play")
            }
            EffectIssueKind::InertPositionOffset => f.write_str("position offsets do nothing in this client"),
            EffectIssueKind::UserPreferenceZeroMask => f.write_str("user preference mask 0 always skips its children"),
            EffectIssueKind::UserPreferenceUnknownBits { mask } => {
                write!(f, "user preference mask {mask:#X} has bits the client never sets: the children always play")
            }
        }
    }
}

/// Validates a definition list (a collection entry or a tileart record's effects).
pub fn validate_effect_definitions(
    definitions: &[EffectDefinition],
    context: &EffectValidationContext<'_>,
) -> Vec<EffectIssue> {
    let mut validator = Validator { context, entry: None, issues: Vec::new() };
    validator.list(definitions, "", 0);
    validator.issues
}

impl EffectDefinitionCollection {
    /// Validates every entry.
    pub fn validate(&self, context: &EffectValidationContext<'_>) -> Vec<EffectIssue> {
        let mut validator = Validator { context, entry: None, issues: Vec::new() };
        for entry in &self.entries {
            validator.entry = Some(entry.key);
            if entry.definitions.is_empty() {
                validator.push("", EffectIssueKind::EmptyEntry);
            }
            validator.list(&entry.definitions, "", 0);
        }
        validator.issues
    }
}

struct Validator<'c, 'a> {
    context: &'c EffectValidationContext<'a>,
    entry: Option<EffectEntryKey>,
    issues: Vec<EffectIssue>,
}

impl Validator<'_, '_> {
    fn push(&mut self, path: &str, kind: EffectIssueKind) {
        self.issues.push(EffectIssue { entry: self.entry, path: path.to_owned(), kind });
    }

    fn list(&mut self, definitions: &[EffectDefinition], prefix: &str, depth: usize) {
        for (index, definition) in definitions.iter().enumerate() {
            let path = if prefix.is_empty() { index.to_string() } else { format!("{prefix}/{index}") };
            self.definition(definition, &path, depth);
        }
    }

    fn map(&mut self, entries: &[EffectMapEntry], prefix: &str, label: &str, depth: usize) {
        let mut seen = Vec::new();
        for entry in entries {
            if seen.contains(&entry.key) {
                self.push(prefix, EffectIssueKind::DuplicateMapKey { key: entry.key });
            }
            seen.push(entry.key);
            self.list(&entry.children, &format!("{prefix}/{label} {}", entry.key), depth);
        }
    }

    fn finite(&mut self, path: &str, field: &'static str, finite: bool) {
        if !finite {
            self.push(path, EffectIssueKind::NonFiniteValue { field });
        }
    }

    fn definition(&mut self, definition: &EffectDefinition, path: &str, depth: usize) {
        if depth > MAX_EFFECT_NESTING_DEPTH {
            self.push(path, EffectIssueKind::NestingTooDeep);
            return;
        }
        let child_depth = depth + 1;
        match definition {
            EffectDefinition::Particle(effect) => {
                for (field, value) in [
                    ("offset_x", effect.offset_x),
                    ("offset_y", effect.offset_y),
                    ("camera_offset", effect.camera_offset),
                    ("scale", effect.scale),
                    ("duration", effect.duration),
                ] {
                    self.finite(path, field, value.is_finite());
                }
                if effect.field_3c != 0 {
                    self.push(path, EffectIssueKind::NonZeroField3c { value: effect.field_3c });
                }
                if effect.attach_mode > PARTICLE_ATTACH_PROJECTILE_ONLY {
                    self.push(path, EffectIssueKind::UnknownAttachMode { value: effect.attach_mode });
                }
                if let Some(strings) = self.context.strings {
                    match effect.name(strings) {
                        None => {
                            self.push(path, EffectIssueKind::UnresolvedParticleName { name_index: effect.name_index })
                        }
                        Some(name) => {
                            if let Some(exists) = self.context.particle_file_exists
                                && !exists(name)
                            {
                                self.push(path, EffectIssueKind::MissingParticleFile { name: name.to_owned() });
                            }
                        }
                    }
                }
            }
            EffectDefinition::Sound(effect) => {
                self.finite(path, "value_08", effect.value_08.is_finite());
                if effect.value_08 != SOUND_VALUE_08_CLIENT {
                    self.push(path, EffectIssueKind::UnusedSoundValue { value: effect.value_08 });
                }
            }
            EffectDefinition::RoundRobin(effect) => self.list(&effect.children, path, child_depth),
            EffectDefinition::PositionOffset(_) => self.push(path, EffectIssueKind::InertPositionOffset),
            EffectDefinition::Wait(effect) => {
                if effect.min_delay_ms > effect.max_delay_ms {
                    let (min, max) = (effect.min_delay_ms, effect.max_delay_ms);
                    self.push(path, EffectIssueKind::WaitMinAboveMax { min, max });
                }
                self.list(&effect.children, path, child_depth);
            }
            EffectDefinition::Lod(effect) => {
                for level in effect.levels.iter().filter(|level| level.key > PARTICLE_LOD_MAX) {
                    self.push(path, EffectIssueKind::LodLevelUnreachable { key: level.key });
                }
                self.map(&effect.levels, path, "level", child_depth);
            }
            EffectDefinition::PropertyFlag(effect) => {
                if effect.mode == PropertyFlagMode::FlagMask {
                    if !effect.default_children.is_empty() {
                        self.push(path, EffectIssueKind::FlagMaskDefaultUnused);
                    }
                    for case in effect.cases.iter().filter(|case| !FLAG_MASK_KEYS.contains(&case.key)) {
                        self.push(path, EffectIssueKind::FlagMaskCaseUnreachable { key: case.key });
                    }
                }
                self.list(&effect.default_children, &format!("{path}/default"), child_depth);
                self.map(&effect.cases, path, "case", child_depth);
            }
            EffectDefinition::Earthquake(effect) => {
                self.finite(path, "amplitude", effect.amplitude.is_finite());
                self.finite(path, "duration", effect.duration.is_finite());
                self.finite(path, "frequency", effect.frequency.is_finite());
            }
            EffectDefinition::Random(effect) => {
                let mut sum = 0.0;
                for entry in &effect.entries {
                    self.finite(path, "weight", entry.weight.is_finite());
                    if entry.weight < 0.0 {
                        self.push(path, EffectIssueKind::NegativeRandomWeight { weight: entry.weight });
                    }
                    sum += entry.weight;
                }
                if sum.is_finite() && (sum - RANDOM_WEIGHT_TOTAL).abs() > RANDOM_WEIGHT_TOLERANCE {
                    self.push(path, EffectIssueKind::RandomWeightSum { sum });
                }
                for (index, entry) in effect.entries.iter().enumerate() {
                    self.list(&entry.children, &format!("{path}/entry {index}"), child_depth);
                }
            }
            EffectDefinition::UserPreference(effect) => {
                if effect.mask == 0 {
                    self.push(path, EffectIssueKind::UserPreferenceZeroMask);
                } else if effect.mask & !USER_PREFERENCE_FOOTSTEPS != 0 {
                    self.push(path, EffectIssueKind::UserPreferenceUnknownBits { mask: effect.mask });
                }
                self.list(&effect.children, path, child_depth);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::{
        EffectCollectionEntry, LodEffect, ParticleEffect, PropertyFlagEffect, RandomEffect, RandomEffectEntry,
        SoundEffect, UserPreferenceEffect, WaitEffect,
    };

    const LOD_BEYOND_MAX: u32 = PARTICLE_LOD_MAX + 1;
    const FLAG_MASK_OTHER_KEY: u32 = 2;
    const UNKNOWN_ATTACH_MODE: u8 = PARTICLE_ATTACH_PROJECTILE_ONLY + 1;

    fn sound(sound_id: u32) -> EffectDefinition {
        EffectDefinition::Sound(SoundEffect { sound_id, value_08: SOUND_VALUE_08_CLIENT })
    }

    fn kinds(issues: &[EffectIssue]) -> Vec<(String, EffectIssueKind)> {
        issues.iter().map(|issue| (issue.path.clone(), issue.kind.clone())).collect()
    }

    #[test]
    fn valid_definitions_have_no_issues() {
        let definitions = [
            sound(1),
            EffectDefinition::Random(RandomEffect {
                entries: vec![
                    RandomEffectEntry { weight: 33.3, children: vec![sound(2)] },
                    RandomEffectEntry { weight: 66.7, children: vec![] },
                ],
            }),
            EffectDefinition::UserPreference(UserPreferenceEffect { mask: USER_PREFERENCE_FOOTSTEPS, children: vec![] }),
        ];
        assert_eq!(validate_effect_definitions(&definitions, &EffectValidationContext::default()), []);
    }

    #[test]
    fn reports_client_behaviour_problems() {
        let definitions = [
            EffectDefinition::Particle(ParticleEffect {
                name_index: 1,
                offset_x: f32::NAN,
                offset_y: 0.0,
                camera_offset: 0.0,
                scale: 1.0,
                attach_mode: UNKNOWN_ATTACH_MODE,
                duration: 0.0,
                looping: false,
                field_3c: 1,
                hued: false,
            }),
            EffectDefinition::Wait(WaitEffect { children: vec![], min_delay_ms: 5, max_delay_ms: 1, aligned: false }),
            EffectDefinition::Lod(LodEffect {
                levels: vec![
                    EffectMapEntry { key: 1, children: vec![] },
                    EffectMapEntry { key: 1, children: vec![sound(0)] },
                    EffectMapEntry { key: LOD_BEYOND_MAX, children: vec![] },
                ],
            }),
            EffectDefinition::PropertyFlag(PropertyFlagEffect {
                mode: PropertyFlagMode::FlagMask,
                mask: 1,
                property: 0,
                default_children: vec![sound(0)],
                cases: vec![EffectMapEntry { key: FLAG_MASK_OTHER_KEY, children: vec![] }],
            }),
            EffectDefinition::Random(RandomEffect {
                entries: vec![RandomEffectEntry { weight: -1.0, children: vec![] }],
            }),
            EffectDefinition::UserPreference(UserPreferenceEffect { mask: 0, children: vec![] }),
        ];
        let issues = validate_effect_definitions(&definitions, &EffectValidationContext::default());
        assert_eq!(
            kinds(&issues),
            [
                ("0".to_owned(), EffectIssueKind::NonFiniteValue { field: "offset_x" }),
                ("0".to_owned(), EffectIssueKind::NonZeroField3c { value: 1 }),
                ("0".to_owned(), EffectIssueKind::UnknownAttachMode { value: UNKNOWN_ATTACH_MODE }),
                ("1".to_owned(), EffectIssueKind::WaitMinAboveMax { min: 5, max: 1 }),
                ("2".to_owned(), EffectIssueKind::LodLevelUnreachable { key: LOD_BEYOND_MAX }),
                ("2".to_owned(), EffectIssueKind::DuplicateMapKey { key: 1 }),
                ("3".to_owned(), EffectIssueKind::FlagMaskDefaultUnused),
                ("3".to_owned(), EffectIssueKind::FlagMaskCaseUnreachable { key: FLAG_MASK_OTHER_KEY }),
                ("4".to_owned(), EffectIssueKind::NegativeRandomWeight { weight: -1.0 }),
                ("4".to_owned(), EffectIssueKind::RandomWeightSum { sum: -1.0 }),
                ("5".to_owned(), EffectIssueKind::UserPreferenceZeroMask),
            ]
        );
        assert_eq!(issues[1].severity(), EffectIssueSeverity::Error);
        assert_eq!(issues[2].severity(), EffectIssueSeverity::Warning);
    }

    #[test]
    fn reports_nested_paths_and_empty_entries() {
        let collection = EffectDefinitionCollection {
            entries: vec![
                EffectCollectionEntry { key: EffectEntryKey::Id(7), definitions: vec![] },
                EffectCollectionEntry {
                    key: EffectEntryKey::Id(8),
                    definitions: vec![EffectDefinition::Random(RandomEffect {
                        entries: vec![RandomEffectEntry {
                            weight: RANDOM_WEIGHT_TOTAL,
                            children: vec![EffectDefinition::Sound(SoundEffect { sound_id: 1, value_08: 0.5 })],
                        }],
                    })],
                },
            ],
        };
        let issues = collection.validate(&EffectValidationContext::default());
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].to_string(), "entry 7: no definitions: plays nothing");
        assert_eq!(issues[1].path, "0/entry 0/0");
        assert_eq!(issues[1].entry, Some(EffectEntryKey::Id(8)));
    }
}
