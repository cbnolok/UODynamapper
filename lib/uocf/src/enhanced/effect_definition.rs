//! Enhanced Client effect definitions (`UOEffects::ParseEffectDefinition` in UOSA.exe).
//!
//! The same recursive format ends every `tileart.uop` record (u8 count, then the definitions)
//! and is the whole payload of every `EffectDefinitionCollection.uop` entry (u32 count, then
//! the definitions). Formats, field meanings and confidence levels follow
//! `scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md`.
//!
//! A definition is a u32 type key followed by a type-specific body. An effect list is a u32
//! count followed by that many definitions; an effect map is a u32 count followed by that many
//! {u32 key, effect list} pairs.

mod collection;
mod encode;
mod kdl_text;
mod validation;

pub use collection::{
    effect_collection_entry_hash, effect_collection_entry_path, EffectCollectionChanges, EffectCollectionEntry,
    EffectDefinitionCollection, EffectEntryKey, EFFECT_COLLECTION_ID_SCAN_END, EFFECT_DEFINITION_COLLECTION_UOP_NAME,
};
pub use encode::{
    encode_effect_definition_collection, encode_effect_definitions, encode_tileart_effects, write_effect_definition,
    write_effect_list, TILEART_EFFECT_COUNT_MAX,
};
pub use kdl_text::{effect_definitions_from_kdl, effect_definitions_to_kdl};
pub use validation::{
    validate_effect_definitions, EffectIssue, EffectIssueKind, EffectIssueSeverity, EffectValidationContext,
    RANDOM_WEIGHT_TOLERANCE,
};

use crate::enhanced::string_dictionary::UoStringDictionary;
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{bail, ensure, Result};
use std::io::Cursor;

/// Type key of a particle effect (`UOParticleEffectDefinition`).
pub const EFFECT_TYPE_PARTICLE: u32 = 0;
/// Type key of a sound effect (`UOSoundEffectDefinition`).
pub const EFFECT_TYPE_SOUND: u32 = 1;
/// Type key of a round-robin collection (`UORoundRobinEffectCollection`).
pub const EFFECT_TYPE_ROUND_ROBIN: u32 = 2;
/// Type key of a per-direction position offset (`UOPositionOffsetDefinition`).
pub const EFFECT_TYPE_POSITION_OFFSET: u32 = 7;
/// Type key of a timer collection (`UOWaitCollectionDefinition`).
pub const EFFECT_TYPE_WAIT: u32 = 10;
/// Type key of a level-of-detail selector (`UOEffectLODDefinition`).
pub const EFFECT_TYPE_LOD: u32 = 11;
/// Type key of an object property selector (`UOEffectPropertyFlagDefinition`).
pub const EFFECT_TYPE_PROPERTY_FLAG: u32 = 12;
/// Type key of a camera shake (`UOEarthquakeEffectDefinition`).
pub const EFFECT_TYPE_EARTHQUAKE: u32 = 15;
/// Type key of a weighted random collection (`UORandomEffectCollectionDefinition`).
pub const EFFECT_TYPE_RANDOM: u32 = 16;
/// Type key of a user effect setting filter (`UOUserPrefEffectFilterDefinition`).
pub const EFFECT_TYPE_USER_PREFERENCE: u32 = 17;

/// Collection id = 0xC7 packet effect field - `EFFECT_ID_PACKET_BIAS`; see
/// `collection_id_from_packet_effect`.
pub const EFFECT_ID_PACKET_BIAS: u32 = 1;
/// Packet effect value that plays nothing.
pub const PACKET_EFFECT_NONE: u16 = 9501;
/// Packet effect value that plays the classic item (tileart) effect instead of a collection
/// entry. Packet effect 0 (also what 0x70 carries) behaves the same.
pub const PACKET_EFFECT_CLASSIC: u16 = 9502;
/// Collection id the client loads instead of a missing id (a `Placeholder01.ems` particle);
/// if it is missing too, nothing plays.
pub const EFFECT_ID_FALLBACK: u32 = 0;
/// Collection id of packet effect type 1 (lightning, `lightning.ems`).
pub const EFFECT_ID_LIGHTNING: u32 = 666;
/// Collection id of the current-target indicator (`Splat_Golden.nif` + `Splat_Splash.nif`).
pub const EFFECT_ID_TARGET_INDICATOR: u32 = 10001;

/// `UserPreferenceEffect::mask` bit set when the user disabled footsteps (user setting
/// `Effects`, `Footsteps.enabled`); the other bits come only from the settings file.
pub const USER_PREFERENCE_FOOTSTEPS: u32 = 0x1;

/// `ParticleEffect::attach_mode` value: the client does not attach the effect to the world
/// object; it plays only while the item is shown in flight (thrown models). Client data: 2 on
/// thrown models, 0 on arrows, 1 elsewhere; the client treats 0 and 1 alike.
pub const PARTICLE_ATTACH_PROJECTILE_ONLY: u8 = 2;

/// Directory of the particle files: the `.nif` or `.ems` file of a particle is the
/// `EffectTexture.uop` entry `PARTICLE_RESOURCE_DIR` + lowercase name.
pub const PARTICLE_RESOURCE_DIR: &str = "data/effects/";

/// Highest particle level of detail the client accepts (settings file `Particles` element,
/// `lod` attribute); the UI Low/Medium/High maps to 1..3.
pub const PARTICLE_LOD_MAX: u32 = 3;
/// Client default particle level of detail.
pub const PARTICLE_LOD_DEFAULT: u32 = 3;

/// Sum of the weights of a random effect that always plays an entry: weights are percentages.
pub const RANDOM_WEIGHT_TOTAL: f32 = 100.0;

/// Directions of a position offset effect, indexed 0..7.
pub const POSITION_OFFSET_DIRECTION_COUNT: usize = 8;

/// Deepest nesting accepted by the readers; top-level definitions have depth 0.
///
/// This limit is ours, not the client's: `UOEffects::ParseEffectDefinition` recurses without a
/// bound. Client data nests at most 1 level deep; the limit only rejects corrupt input before
/// it exhausts the stack.
pub const MAX_EFFECT_NESTING_DEPTH: usize = 16;

/// Smallest encoded definition, map entry or random entry: a u32 (type key, map key or weight)
/// followed by the u32 count of an empty list. Counts that cannot fit in the bytes left are
/// rejected before allocating.
const MIN_EFFECT_ELEMENT_SIZE: u64 = 8;

/// Byte values of the flags the client accepts only as 0 or 1 (Wait `aligned`, PropertyFlag `mode`).
const STRICT_FLAG_FALSE: u8 = 0;
const STRICT_FLAG_TRUE: u8 = 1;

/// The repo's string dictionary starts one string after the client's (its 16-byte header
/// swallows the client's empty string 0), so client index `n` is dictionary index `n - 1`.
const STRING_DICTIONARY_INDEX_SHIFT: u32 = 1;

/// One effect definition, by type key.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectDefinition {
    Particle(ParticleEffect),
    Sound(SoundEffect),
    RoundRobin(RoundRobinEffect),
    PositionOffset(PositionOffsetEffect),
    Wait(WaitEffect),
    Lod(LodEffect),
    PropertyFlag(PropertyFlagEffect),
    Earthquake(EarthquakeEffect),
    Random(RandomEffect),
    UserPreference(UserPreferenceEffect),
}

/// Key 0: a `.nif` or `.ems` particle scene. A name containing `.ems` instantiates
/// `UOParticleEffectChaos`, otherwise a (hued) `UOParticleEffectNif`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEffect {
    /// Client string dictionary index of the `.nif` or `.ems` file name (confirmed);
    /// see `name`. The file is the `EffectTexture.uop` entry `PARTICLE_RESOURCE_DIR` +
    /// lowercase name; `.ems` files are Chaos text scripts naming further `.nif`/`.tga` files
    /// there.
    pub name_index: u32,
    /// Added to the effect x (probable).
    pub offset_x: f32,
    /// Subtracted from the effect z, so it acts as a vertical offset (probable).
    pub offset_y: f32,
    /// Moves the effect along the camera vector by `-camera_offset * 0.12856` (probable).
    pub camera_offset: f32,
    /// Scene node scale (confirmed).
    pub scale: f32,
    /// Attach mode (confirmed for `PARTICLE_ATTACH_PROJECTILE_ONLY`); see the constant.
    pub attach_mode: u8,
    /// Seconds (confirmed): a non-looping effect stops once elapsed >= duration, a looping one
    /// wraps the elapsed time when duration is non-zero. Independent of the packet duration,
    /// which the client applies as an update-tick countdown (single-definition entries only).
    pub duration: f32,
    /// Restarts the effect when it ends; any non-zero byte is true (confirmed).
    pub looping: bool,
    /// Must be 0: the client refuses to start a Nif effect when it is 1 and a Chaos (`.ems`)
    /// effect when it is non-zero (confirmed); 0 in all client data. Meaning of other values
    /// unknown.
    pub field_3c: u8,
    /// Uses `UOHuedParticleEffectNif` when the object hue is non-zero; any non-zero byte is
    /// true (confirmed).
    pub hued: bool,
}

/// Key 1: a sound played at the effect position.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundEffect {
    /// Sound id played with distance attenuation (confirmed).
    pub sound_id: u32,
    /// Copied into the sound instance but never read by the client (confirmed); 1.0 in all
    /// client data, probably an intended gain.
    pub value_08: f32,
}

/// Key 2: instantiates one child per use, cycling through them in order (confirmed).
///
/// The cycle index belongs to the definition: it is shared by every object of the same graphic
/// and every play of the same collection entry.
#[derive(Debug, Clone, PartialEq)]
pub struct RoundRobinEffect {
    pub children: Vec<EffectDefinition>,
}

/// Position offset of one direction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DirectionOffset {
    pub x: i8,
    pub y: i8,
}

/// Key 7: per-direction position offset (confirmed).
///
/// Inert in this client build: the runtime object stores the offsets but nothing reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionOffsetEffect {
    /// Offsets indexed by direction.
    pub offsets: [DirectionOffset; POSITION_OFFSET_DIRECTION_COUNT],
}

/// Key 10: throttle (confirmed). When instantiated, plays its children only if the
/// per-definition next time has passed. It then sets the next time to now + `min_delay_ms`
/// (next multiple of `min_delay_ms` when `aligned`) + uniform [0, max - min] ms. It never fires
/// again by itself.
#[derive(Debug, Clone, PartialEq)]
pub struct WaitEffect {
    pub children: Vec<EffectDefinition>,
    /// Lower bound of the random delay before the next fire (client default 2000).
    pub min_delay_ms: u32,
    /// Upper bound of the random delay before the next fire (client default 4000).
    pub max_delay_ms: u32,
    /// Aligns the next fire time to a multiple of `min_delay_ms` (confirmed).
    pub aligned: bool,
}

/// One {key, effect list} pair of an effect map, in file order.
///
/// The client stores maps in a `std::map`, so a repeated key keeps its last list and iteration
/// is in ascending key order.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectMapEntry {
    pub key: u32,
    pub children: Vec<EffectDefinition>,
}

/// Key 11: plays the entry for the particle LOD setting (0..=`PARTICLE_LOD_MAX`, client default
/// `PARTICLE_LOD_DEFAULT`; UI Low/Medium/High = 1..3), falling back to the next lower key;
/// nothing if no key is <= the setting (confirmed).
#[derive(Debug, Clone, PartialEq)]
pub struct LodEffect {
    pub levels: Vec<EffectMapEntry>,
}

/// How a property flag effect computes the key it looks up in `cases`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PropertyFlagMode {
    /// The key is `(object flags & mask) != 0`; no match produces nothing.
    FlagMask = STRICT_FLAG_FALSE,
    /// The key is the object value selected by `property`; no match uses `default_children`.
    Property = STRICT_FLAG_TRUE,
}

/// Key 12: selects children by an object property (confirmed).
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyFlagEffect {
    pub mode: PropertyFlagMode,
    /// Object flag mask of `PropertyFlagMode::FlagMask`.
    pub mask: u64,
    /// Object value of `PropertyFlagMode::Property`: 2 = vt+0x80 & 0xffff, 3 = vt+0xb4,
    /// 7 = object hue, 8 = vt+0x68, any other value = that tileart props1 key.
    pub property: u32,
    /// Children used when no case matches in `PropertyFlagMode::Property`.
    pub default_children: Vec<EffectDefinition>,
    pub cases: Vec<EffectMapEntry>,
}

/// Key 15: shakes the camera target x by `amplitude * sin(elapsed * frequency * 2pi)` (confirmed).
#[derive(Debug, Clone, PartialEq)]
pub struct EarthquakeEffect {
    pub amplitude: f64,
    /// Seconds; the effect is finished when the elapsed time exceeds it.
    pub duration: f64,
    /// Hz.
    pub frequency: f64,
}

/// One weighted entry of a random collection.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomEffectEntry {
    /// Percent (confirmed): the client draws r in [0, `RANDOM_WEIGHT_TOTAL`) and plays the
    /// first entry with r < weight after subtracting the previous weights. A sum below
    /// `RANDOM_WEIGHT_TOTAL` leaves a chance of nothing.
    pub weight: f32,
    pub children: Vec<EffectDefinition>,
}

/// Key 16: instantiates one entry, picked by weight.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomEffect {
    pub entries: Vec<RandomEffectEntry>,
}

/// Key 17: filters its children by the user effect settings (confirmed).
#[derive(Debug, Clone, PartialEq)]
pub struct UserPreferenceEffect {
    /// The children are skipped when all of these user effect setting bits are set, or when it
    /// is 1 and object vt+0xa0 is negative. Bit 0 is `USER_PREFERENCE_FOOTSTEPS`; the client
    /// only ever sets bit 0, so the other bits are never set by the client. No client tileart or
    /// collection effect uses this type.
    pub mask: u32,
    pub children: Vec<EffectDefinition>,
}

impl EffectDefinition {
    /// Type key as stored in the file.
    pub fn type_key(&self) -> u32 {
        match self {
            Self::Particle(_) => EFFECT_TYPE_PARTICLE,
            Self::Sound(_) => EFFECT_TYPE_SOUND,
            Self::RoundRobin(_) => EFFECT_TYPE_ROUND_ROBIN,
            Self::PositionOffset(_) => EFFECT_TYPE_POSITION_OFFSET,
            Self::Wait(_) => EFFECT_TYPE_WAIT,
            Self::Lod(_) => EFFECT_TYPE_LOD,
            Self::PropertyFlag(_) => EFFECT_TYPE_PROPERTY_FLAG,
            Self::Earthquake(_) => EFFECT_TYPE_EARTHQUAKE,
            Self::Random(_) => EFFECT_TYPE_RANDOM,
            Self::UserPreference(_) => EFFECT_TYPE_USER_PREFERENCE,
        }
    }

    /// Calls `visitor` on this definition and then on every nested definition, depth first.
    pub fn visit(&self, visitor: &mut dyn FnMut(&EffectDefinition)) {
        visitor(self);
        fn visit_list(children: &[EffectDefinition], visitor: &mut dyn FnMut(&EffectDefinition)) {
            children.iter().for_each(|child| child.visit(visitor));
        }
        match self {
            Self::Particle(_) | Self::Sound(_) | Self::PositionOffset(_) | Self::Earthquake(_) => {}
            Self::RoundRobin(effect) => visit_list(&effect.children, visitor),
            Self::Wait(effect) => visit_list(&effect.children, visitor),
            Self::Lod(effect) => effect.levels.iter().for_each(|level| visit_list(&level.children, visitor)),
            Self::PropertyFlag(effect) => {
                visit_list(&effect.default_children, visitor);
                effect.cases.iter().for_each(|case| visit_list(&case.children, visitor));
            }
            Self::Random(effect) => effect.entries.iter().for_each(|entry| visit_list(&entry.children, visitor)),
            Self::UserPreference(effect) => visit_list(&effect.children, visitor),
        }
    }
}

impl ParticleEffect {
    /// Resolves `name_index` in the repo's string dictionary.
    pub fn name<'a>(&self, string_dictionary: &'a UoStringDictionary) -> Option<&'a str> {
        self.name_index
            .checked_sub(STRING_DICTIONARY_INDEX_SHIFT)
            .and_then(|index| string_dictionary.get_string(index as usize))
    }
}

/// Reads one definition: a u32 type key, then its body.
pub fn read_effect_definition(reader: &mut Cursor<&[u8]>) -> Result<EffectDefinition> {
    read_definition(reader, 0)
}

/// Reads `count` definitions, as stored after the count of a tileart record tail or of a
/// collection entry.
pub fn read_effect_definitions(reader: &mut Cursor<&[u8]>, count: usize) -> Result<Vec<EffectDefinition>> {
    read_definitions(reader, count, 0)
}

/// Reads an effect list (`UOEffects::ParseEffectList`): u32 count, then that many definitions.
pub fn read_effect_list(reader: &mut Cursor<&[u8]>) -> Result<Vec<EffectDefinition>> {
    read_list(reader, 0)
}

/// Parses the payload of one `EffectDefinitionCollection.uop` entry
/// (`UOEffects::ParseEffectDefinitionCollection`): u32 count, then that many definitions,
/// ending the payload exactly.
///
/// All top-level definitions play at the same time (one container); there is no sequencing.
/// A missing entry falls back to `EFFECT_ID_FALLBACK`.
pub fn parse_effect_definition_collection(data: &[u8]) -> Result<Vec<EffectDefinition>> {
    let mut reader = Cursor::new(data);
    let definitions = read_effect_list(&mut reader)?;
    let trailing = data.len() - reader.position() as usize;
    ensure!(trailing == 0, "{trailing} trailing bytes after the effect definition collection");
    Ok(definitions)
}

/// Maps the effect field of a 0xC7 packet (fixed location or fixed on mobile) to the
/// `EffectDefinitionCollection.uop` entry id the client plays.
///
/// Returns `None` when the client plays no collection entry: `PACKET_EFFECT_NONE` plays
/// nothing, while 0 and `PACKET_EFFECT_CLASSIC` play the classic item effect instead.
pub fn collection_id_from_packet_effect(effect: u16) -> Option<u32> {
    match effect {
        PACKET_EFFECT_NONE | PACKET_EFFECT_CLASSIC => None,
        // Effect 0 is not id -1: the subtraction fails exactly for the classic item effect.
        _ => u32::from(effect).checked_sub(EFFECT_ID_PACKET_BIAS),
    }
}

fn read_definition(reader: &mut Cursor<&[u8]>, depth: usize) -> Result<EffectDefinition> {
    let offset = reader.position();
    ensure!(
        depth <= MAX_EFFECT_NESTING_DEPTH,
        "effect definition at offset {offset} exceeds the nesting depth limit {MAX_EFFECT_NESTING_DEPTH}"
    );
    let child_depth = depth + 1;
    let definition = match reader.read_u32::<LittleEndian>()? {
        EFFECT_TYPE_PARTICLE => EffectDefinition::Particle(ParticleEffect {
            name_index: reader.read_u32::<LittleEndian>()?,
            offset_x: reader.read_f32::<LittleEndian>()?,
            offset_y: reader.read_f32::<LittleEndian>()?,
            camera_offset: reader.read_f32::<LittleEndian>()?,
            scale: reader.read_f32::<LittleEndian>()?,
            attach_mode: reader.read_u8()?,
            duration: reader.read_f32::<LittleEndian>()?,
            looping: reader.read_u8()? != 0,
            field_3c: reader.read_u8()?,
            hued: reader.read_u8()? != 0,
        }),
        EFFECT_TYPE_SOUND => EffectDefinition::Sound(SoundEffect {
            sound_id: reader.read_u32::<LittleEndian>()?,
            value_08: reader.read_f32::<LittleEndian>()?,
        }),
        EFFECT_TYPE_ROUND_ROBIN => EffectDefinition::RoundRobin(RoundRobinEffect {
            children: read_list(reader, child_depth)?,
        }),
        EFFECT_TYPE_POSITION_OFFSET => {
            let mut offsets = [DirectionOffset::default(); POSITION_OFFSET_DIRECTION_COUNT];
            for offset in &mut offsets {
                offset.x = reader.read_i8()?;
                offset.y = reader.read_i8()?;
            }
            EffectDefinition::PositionOffset(PositionOffsetEffect { offsets })
        }
        EFFECT_TYPE_WAIT => EffectDefinition::Wait(WaitEffect {
            children: read_list(reader, child_depth)?,
            min_delay_ms: reader.read_u32::<LittleEndian>()?,
            max_delay_ms: reader.read_u32::<LittleEndian>()?,
            aligned: read_strict_flag(reader, "wait effect aligned flag")?,
        }),
        EFFECT_TYPE_LOD => EffectDefinition::Lod(LodEffect {
            levels: read_map(reader, child_depth)?,
        }),
        EFFECT_TYPE_PROPERTY_FLAG => EffectDefinition::PropertyFlag(PropertyFlagEffect {
            mode: if read_strict_flag(reader, "property flag effect mode")? {
                PropertyFlagMode::Property
            } else {
                PropertyFlagMode::FlagMask
            },
            mask: reader.read_u64::<LittleEndian>()?,
            property: reader.read_u32::<LittleEndian>()?,
            default_children: read_list(reader, child_depth)?,
            cases: read_map(reader, child_depth)?,
        }),
        EFFECT_TYPE_EARTHQUAKE => EffectDefinition::Earthquake(EarthquakeEffect {
            amplitude: reader.read_f64::<LittleEndian>()?,
            duration: reader.read_f64::<LittleEndian>()?,
            frequency: reader.read_f64::<LittleEndian>()?,
        }),
        EFFECT_TYPE_RANDOM => {
            let count = read_count(reader, "random effect entry")?;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                let weight = reader.read_f32::<LittleEndian>()?;
                let children = read_list(reader, child_depth)?;
                entries.push(RandomEffectEntry { weight, children });
            }
            EffectDefinition::Random(RandomEffect { entries })
        }
        EFFECT_TYPE_USER_PREFERENCE => EffectDefinition::UserPreference(UserPreferenceEffect {
            mask: reader.read_u32::<LittleEndian>()?,
            children: read_list(reader, child_depth)?,
        }),
        type_key => bail!("unknown effect type key {type_key} at offset {offset}"),
    };
    Ok(definition)
}

fn read_definitions(reader: &mut Cursor<&[u8]>, count: usize, depth: usize) -> Result<Vec<EffectDefinition>> {
    ensure_count_fits(reader, count, "effect definition")?;
    let mut definitions = Vec::with_capacity(count);
    for _ in 0..count {
        definitions.push(read_definition(reader, depth)?);
    }
    Ok(definitions)
}

fn read_list(reader: &mut Cursor<&[u8]>, depth: usize) -> Result<Vec<EffectDefinition>> {
    let count = reader.read_u32::<LittleEndian>()? as usize;
    read_definitions(reader, count, depth)
}

/// Reads an effect map (`UOEffects::ParseEffectMap`): u32 count, then that many {u32 key, list}.
fn read_map(reader: &mut Cursor<&[u8]>, depth: usize) -> Result<Vec<EffectMapEntry>> {
    let count = read_count(reader, "effect map entry")?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let key = reader.read_u32::<LittleEndian>()?;
        let children = read_list(reader, depth)?;
        entries.push(EffectMapEntry { key, children });
    }
    Ok(entries)
}

fn read_count(reader: &mut Cursor<&[u8]>, element: &str) -> Result<usize> {
    let count = reader.read_u32::<LittleEndian>()? as usize;
    ensure_count_fits(reader, count, element)?;
    Ok(count)
}

fn ensure_count_fits(reader: &Cursor<&[u8]>, count: usize, element: &str) -> Result<()> {
    let remaining = (reader.get_ref().len() as u64).saturating_sub(reader.position());
    ensure!(
        count as u64 * MIN_EFFECT_ELEMENT_SIZE <= remaining,
        "{element} count {count} at offset {} cannot fit in the {remaining} bytes left",
        reader.position()
    );
    Ok(())
}

/// Reads a byte the client accepts only as 0 or 1 (it clears its parse ok flag otherwise).
fn read_strict_flag(reader: &mut Cursor<&[u8]>, field: &str) -> Result<bool> {
    match reader.read_u8()? {
        STRICT_FLAG_FALSE => Ok(false),
        STRICT_FLAG_TRUE => Ok(true),
        value => bail!("invalid {field} {value} (the client accepts only 0 or 1)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Little-endian writer for synthetic effect bytes.
    #[derive(Default)]
    struct EffectWriter(Vec<u8>);

    impl EffectWriter {
        fn u8(&mut self, value: u8) -> &mut Self { self.0.push(value); self }
        fn i8(&mut self, value: i8) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn u32(&mut self, value: u32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn u64(&mut self, value: u64) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn f32(&mut self, value: f32) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn f64(&mut self, value: f64) -> &mut Self { self.0.extend_from_slice(&value.to_le_bytes()); self }
        fn sound(&mut self, sound_id: u32) -> &mut Self { self.u32(EFFECT_TYPE_SOUND).u32(sound_id).f32(1.0) }
    }

    fn sound(sound_id: u32) -> EffectDefinition {
        EffectDefinition::Sound(SoundEffect { sound_id, value_08: 1.0 })
    }

    /// Decodes `bytes` as one definition that must consume them all.
    fn decode(bytes: &[u8]) -> Result<EffectDefinition> {
        let mut reader = Cursor::new(bytes);
        let definition = read_effect_definition(&mut reader)?;
        assert_eq!(reader.position() as usize, bytes.len(), "definition left bytes unread");
        Ok(definition)
    }

    #[test]
    fn effect_reads_particle() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_PARTICLE).u32(42).f32(-8.0).f32(4.0).f32(-10.0).f32(0.25)
            .u8(PARTICLE_ATTACH_PROJECTILE_ONLY).f32(3.0).u8(7).u8(0).u8(1);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::Particle(ParticleEffect {
            name_index: 42, offset_x: -8.0, offset_y: 4.0, camera_offset: -10.0, scale: 0.25,
            attach_mode: PARTICLE_ATTACH_PROJECTILE_ONLY, duration: 3.0, looping: true, field_3c: 0, hued: true,
        }));
    }

    #[test]
    fn effect_reads_sound() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_SOUND).u32(43).f32(0.5);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::Sound(SoundEffect { sound_id: 43, value_08: 0.5 }));
    }

    #[test]
    fn effect_reads_round_robin_children() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_ROUND_ROBIN).u32(2).sound(1).sound(2);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::RoundRobin(RoundRobinEffect {
            children: vec![sound(1), sound(2)],
        }));
    }

    #[test]
    fn effect_reads_position_offsets_per_direction() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_POSITION_OFFSET);
        for direction in 0..POSITION_OFFSET_DIRECTION_COUNT as i8 {
            writer.i8(direction).i8(-direction);
        }
        let EffectDefinition::PositionOffset(effect) = decode(&writer.0).unwrap() else { panic!("not a position offset") };
        assert_eq!(effect.offsets[3], DirectionOffset { x: 3, y: -3 });
        assert_eq!(effect.offsets[POSITION_OFFSET_DIRECTION_COUNT - 1], DirectionOffset { x: 7, y: -7 });
    }

    #[test]
    fn effect_reads_wait() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_WAIT).u32(2).sound(43).sound(44).u32(500).u32(2000).u8(STRICT_FLAG_TRUE);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::Wait(WaitEffect {
            children: vec![sound(43), sound(44)], min_delay_ms: 500, max_delay_ms: 2000, aligned: true,
        }));
    }

    #[test]
    fn effect_reads_lod_map() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_LOD).u32(2).u32(1).u32(1).sound(1).u32(3).u32(0);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::Lod(LodEffect { levels: vec![
            EffectMapEntry { key: 1, children: vec![sound(1)] },
            EffectMapEntry { key: 3, children: vec![] },
        ] }));
    }

    #[test]
    fn effect_reads_property_flag() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_PROPERTY_FLAG).u8(STRICT_FLAG_TRUE).u64(0x40).u32(7)
            .u32(1).sound(1)
            .u32(1).u32(33).u32(1).sound(2);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::PropertyFlag(PropertyFlagEffect {
            mode: PropertyFlagMode::Property, mask: 0x40, property: 7,
            default_children: vec![sound(1)],
            cases: vec![EffectMapEntry { key: 33, children: vec![sound(2)] }],
        }));
    }

    #[test]
    fn effect_reads_earthquake() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_EARTHQUAKE).f64(0.1).f64(1.0).f64(10.0);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::Earthquake(EarthquakeEffect {
            amplitude: 0.1, duration: 1.0, frequency: 10.0,
        }));
    }

    #[test]
    fn effect_reads_random_with_nested_collections() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_RANDOM).u32(2)
            .f32(0.75).u32(1).u32(EFFECT_TYPE_ROUND_ROBIN).u32(1).sound(1)
            .f32(0.25).u32(0);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::Random(RandomEffect { entries: vec![
            RandomEffectEntry { weight: 0.75, children: vec![
                EffectDefinition::RoundRobin(RoundRobinEffect { children: vec![sound(1)] }),
            ] },
            RandomEffectEntry { weight: 0.25, children: vec![] },
        ] }));
    }

    #[test]
    fn effect_reads_user_preference() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_USER_PREFERENCE).u32(USER_PREFERENCE_FOOTSTEPS).u32(1).sound(5);
        assert_eq!(decode(&writer.0).unwrap(), EffectDefinition::UserPreference(UserPreferenceEffect {
            mask: USER_PREFERENCE_FOOTSTEPS, children: vec![sound(5)],
        }));
    }

    #[test]
    fn effect_visit_reaches_every_nested_definition() {
        let mut writer = EffectWriter::default();
        writer.u32(EFFECT_TYPE_PROPERTY_FLAG).u8(STRICT_FLAG_FALSE).u64(1).u32(0)
            .u32(1).sound(1)
            .u32(1).u32(1).u32(1).u32(EFFECT_TYPE_WAIT).u32(1).sound(2).u32(0).u32(0).u8(STRICT_FLAG_FALSE);
        let mut keys = Vec::new();
        decode(&writer.0).unwrap().visit(&mut |definition| keys.push(definition.type_key()));
        assert_eq!(keys, [EFFECT_TYPE_PROPERTY_FLAG, EFFECT_TYPE_SOUND, EFFECT_TYPE_WAIT, EFFECT_TYPE_SOUND]);
    }

    #[test]
    fn effect_rejects_what_the_client_rejects() {
        let unknown_key = EFFECT_TYPE_POSITION_OFFSET - 1;
        let invalid_flag = STRICT_FLAG_TRUE + 1;
        let mut unknown = EffectWriter::default();
        unknown.u32(unknown_key).u32(0);
        let mut wait = EffectWriter::default();
        wait.u32(EFFECT_TYPE_WAIT).u32(0).u32(0).u32(0).u8(invalid_flag);
        let mut property_flag = EffectWriter::default();
        property_flag.u32(EFFECT_TYPE_PROPERTY_FLAG).u8(invalid_flag).u64(0).u32(0).u32(0).u32(0);
        for (writer, reason) in [
            (unknown, "unknown effect type key"),
            (wait, "wait effect aligned flag"),
            (property_flag, "property flag effect mode"),
        ] {
            let error = read_effect_definition(&mut Cursor::new(&writer.0[..])).expect_err(reason);
            assert!(error.to_string().contains(reason), "{error}");
        }
    }

    #[test]
    fn effect_rejects_nesting_beyond_the_limit_and_counts_beyond_the_data() {
        let nested = |depth: usize| {
            let mut writer = EffectWriter::default();
            (0..depth).for_each(|_| { writer.u32(EFFECT_TYPE_ROUND_ROBIN).u32(1); });
            writer.sound(1);
            writer.0
        };
        assert!(decode(&nested(MAX_EFFECT_NESTING_DEPTH)).is_ok());
        let error = decode(&nested(MAX_EFFECT_NESTING_DEPTH + 1)).expect_err("too deep");
        assert!(error.to_string().contains("nesting depth limit"), "{error}");

        let mut oversized = EffectWriter::default();
        oversized.u32(EFFECT_TYPE_ROUND_ROBIN).u32(u32::MAX);
        let error = decode(&oversized.0).expect_err("oversized count");
        assert!(error.to_string().contains("cannot fit"), "{error}");
    }

    #[test]
    fn effect_collection_must_end_after_its_definitions() {
        let mut writer = EffectWriter::default();
        writer.u32(2).sound(1).u32(EFFECT_TYPE_EARTHQUAKE).f64(0.1).f64(1.0).f64(10.0);
        let definitions = parse_effect_definition_collection(&writer.0).expect("parse collection");
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0], sound(1));

        writer.u8(0);
        let error = parse_effect_definition_collection(&writer.0).expect_err("trailing byte");
        assert!(error.to_string().contains("trailing bytes"), "{error}");
    }

    #[test]
    fn packet_effect_maps_to_collection_id() {
        let (clumsy_packet_effect, clumsy_collection_id) = (5002, 5001);
        let classic_item_effect = 0;
        assert_eq!(collection_id_from_packet_effect(clumsy_packet_effect), Some(clumsy_collection_id));
        assert_eq!(collection_id_from_packet_effect(EFFECT_ID_PACKET_BIAS as u16), Some(EFFECT_ID_FALLBACK));
        assert_eq!(collection_id_from_packet_effect(u16::MAX), Some(u32::from(u16::MAX) - EFFECT_ID_PACKET_BIAS));
        assert_eq!(collection_id_from_packet_effect(classic_item_effect), None);
        assert_eq!(collection_id_from_packet_effect(PACKET_EFFECT_NONE), None);
        assert_eq!(collection_id_from_packet_effect(PACKET_EFFECT_CLASSIC), None);
        // Collection 9501 and 9502 are reached one packet value higher.
        assert_eq!(collection_id_from_packet_effect(PACKET_EFFECT_CLASSIC + 1), Some(u32::from(PACKET_EFFECT_CLASSIC)));
    }

    #[test]
    fn particle_name_resolves_through_the_string_dictionary() {
        let strings = ["first.nif", "second.ems"];
        let mut payload = EffectWriter::default();
        payload.u64(0).u32(strings.len() as u32).u32(0);
        for value in strings {
            payload.0.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.0.extend_from_slice(value.as_bytes());
        }
        let dictionary = UoStringDictionary::from_payload_bytes(&payload.0, "test dictionary").unwrap();
        let particle = |name_index| ParticleEffect {
            name_index, offset_x: 0.0, offset_y: 0.0, camera_offset: 0.0, scale: 1.0,
            attach_mode: 1, duration: 0.0, looping: false, field_3c: 0, hued: false,
        };
        assert_eq!(particle(2).name(&dictionary), Some("second.ems"));
        assert_eq!(particle(0).name(&dictionary), None);
    }
}
