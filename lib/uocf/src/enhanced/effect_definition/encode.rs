//! Serializers of effect definitions: the exact inverse of the readers (flags are written as 0 or 1).
//!
//! The `*_dialect` forms write the grammar of a client; the Kingdom Reborn one fails on what it cannot hold (a
//! `hued` particle, a property flag mask above u32).

use color_eyre::eyre::{self, Result};

use super::{EffectDefinition, EffectMapEntry};
use crate::enhanced::client_dialect::ClientDialect;

/// Largest definition count of a tileart record, whose count is a u8.
pub const TILEART_EFFECT_COUNT_MAX: usize = u8::MAX as usize;

/// The Enhanced Client grammar holds every definition.
const ENHANCED_WRITES_ALL: &str = "the Enhanced Client effect grammar holds every definition";

/// Appends one definition: its u32 type key, then its body.
pub fn write_effect_definition(out: &mut Vec<u8>, definition: &EffectDefinition) {
    write_effect_definition_dialect(out, definition, ClientDialect::Enhanced).expect(ENHANCED_WRITES_ALL);
}

/// [`write_effect_definition`] in the grammar of `dialect`.
pub fn write_effect_definition_dialect(
    out: &mut Vec<u8>,
    definition: &EffectDefinition,
    dialect: ClientDialect,
) -> Result<()> {
    let kr = dialect.is_kingdom_reborn();
    put_u32(out, definition.type_key_in(dialect));
    match definition {
        EffectDefinition::Particle(effect) => {
            put_u32(out, effect.name_index);
            put_f32(out, effect.offset_x);
            put_f32(out, effect.offset_y);
            put_f32(out, effect.camera_offset);
            put_f32(out, effect.scale);
            out.push(effect.attach_mode);
            put_f32(out, effect.duration);
            out.push(u8::from(effect.looping));
            out.push(effect.field_3c);
            if kr {
                if effect.hued {
                    eyre::bail!("a Kingdom Reborn particle cannot be hued");
                }
            } else {
                out.push(u8::from(effect.hued));
            }
        }
        EffectDefinition::Sound(effect) => {
            put_u32(out, effect.sound_id);
            put_f32(out, effect.value_08);
        }
        EffectDefinition::RoundRobin(effect) => write_effect_list_dialect(out, &effect.children, dialect)?,
        EffectDefinition::PositionOffset(effect) => {
            for offset in &effect.offsets {
                out.extend_from_slice(&offset.x.to_le_bytes());
                out.extend_from_slice(&offset.y.to_le_bytes());
            }
        }
        EffectDefinition::Wait(effect) => {
            write_effect_list_dialect(out, &effect.children, dialect)?;
            put_u32(out, effect.min_delay_ms);
            put_u32(out, effect.max_delay_ms);
            out.push(u8::from(effect.aligned));
        }
        EffectDefinition::Lod(effect) => write_map(out, &effect.levels, dialect)?,
        EffectDefinition::PropertyFlag(effect) => {
            out.push(effect.mode as u8);
            if kr {
                let Ok(mask) = u32::try_from(effect.mask) else {
                    eyre::bail!("property flag mask {:#x} does not fit the Kingdom Reborn u32", effect.mask);
                };
                put_u32(out, mask);
            } else {
                out.extend_from_slice(&effect.mask.to_le_bytes());
            }
            put_u32(out, effect.property);
            write_effect_list_dialect(out, &effect.default_children, dialect)?;
            write_map(out, &effect.cases, dialect)?;
        }
        EffectDefinition::Earthquake(effect) => {
            for value in [effect.amplitude, effect.duration, effect.frequency] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        EffectDefinition::Random(effect) => {
            put_count(out, effect.entries.len());
            for entry in &effect.entries {
                put_f32(out, entry.weight);
                write_effect_list_dialect(out, &entry.children, dialect)?;
            }
        }
        EffectDefinition::UserPreference(effect) => {
            put_u32(out, effect.mask);
            write_effect_list_dialect(out, &effect.children, dialect)?;
        }
    }
    Ok(())
}

/// The definitions one after the other, without a count (the inverse of `read_effect_definitions`).
pub fn encode_effect_definitions(definitions: &[EffectDefinition]) -> Vec<u8> {
    encode_effect_definitions_dialect(definitions, ClientDialect::Enhanced).expect(ENHANCED_WRITES_ALL)
}

/// [`encode_effect_definitions`] in the grammar of `dialect`.
pub fn encode_effect_definitions_dialect(definitions: &[EffectDefinition], dialect: ClientDialect) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for definition in definitions {
        write_effect_definition_dialect(&mut out, definition, dialect)?;
    }
    Ok(out)
}

/// Appends an effect list: u32 count, then the definitions.
pub fn write_effect_list(out: &mut Vec<u8>, definitions: &[EffectDefinition]) {
    write_effect_list_dialect(out, definitions, ClientDialect::Enhanced).expect(ENHANCED_WRITES_ALL);
}

/// [`write_effect_list`] in the grammar of `dialect`.
pub fn write_effect_list_dialect(out: &mut Vec<u8>, definitions: &[EffectDefinition], dialect: ClientDialect) -> Result<()> {
    put_count(out, definitions.len());
    for definition in definitions {
        write_effect_definition_dialect(out, definition, dialect)?;
    }
    Ok(())
}

/// The payload of one `EffectDefinitionCollection.uop` entry, the inverse of
/// `parse_effect_definition_collection`.
pub fn encode_effect_definition_collection(definitions: &[EffectDefinition]) -> Vec<u8> {
    encode_effect_definition_collection_dialect(definitions, ClientDialect::Enhanced).expect(ENHANCED_WRITES_ALL)
}

/// [`encode_effect_definition_collection`] in the grammar of `dialect`.
pub fn encode_effect_definition_collection_dialect(
    definitions: &[EffectDefinition],
    dialect: ClientDialect,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_effect_list_dialect(&mut out, definitions, dialect)?;
    Ok(out)
}

/// The effect tail of a `tileart.uop` record: u8 count, then the definitions. Fails above
/// [`TILEART_EFFECT_COUNT_MAX`] definitions.
pub fn encode_tileart_effects(definitions: &[EffectDefinition]) -> Result<Vec<u8>> {
    encode_tileart_effects_dialect(definitions, ClientDialect::Enhanced)
}

/// [`encode_tileart_effects`] in the grammar of `dialect`.
pub fn encode_tileart_effects_dialect(definitions: &[EffectDefinition], dialect: ClientDialect) -> Result<Vec<u8>> {
    let Ok(count) = u8::try_from(definitions.len()) else {
        eyre::bail!(
            "{} effect definitions do not fit a tileart record (at most {TILEART_EFFECT_COUNT_MAX})",
            definitions.len()
        );
    };
    let mut out = vec![count];
    out.extend(encode_effect_definitions_dialect(definitions, dialect)?);
    Ok(out)
}

fn write_map(out: &mut Vec<u8>, entries: &[EffectMapEntry], dialect: ClientDialect) -> Result<()> {
    put_count(out, entries.len());
    for entry in entries {
        put_u32(out, entry.key);
        write_effect_list_dialect(out, &entry.children, dialect)?;
    }
    Ok(())
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// A u32 count. Lists longer than u32::MAX cannot come from the readers or the KDL form.
fn put_count(out: &mut Vec<u8>, count: usize) {
    put_u32(out, u32::try_from(count).expect("an effect list longer than u32::MAX"));
}

/// One definition of every type, nested, with non-default and edge values.
#[cfg(test)]
pub(super) fn every_definition_type() -> Vec<EffectDefinition> {
    use super::*;

    const NAME_INDEX: u32 = 110_239;
    const SOUND_ID: u32 = 43;
    const FLAG_MASK: u64 = 0x8000_0000_0000_0040;
    const HUE_PROPERTY: u32 = 7;
    const CASE_KEY: u32 = 33;

    let sound = |sound_id| EffectDefinition::Sound(SoundEffect { sound_id, value_08: 1.0 });
    let particle = ParticleEffect {
        name_index: NAME_INDEX,
        offset_x: -8.5,
        offset_y: -0.0,
        camera_offset: 1.0e-7,
        scale: 0.25,
        attach_mode: PARTICLE_ATTACH_PROJECTILE_ONLY,
        duration: 3.0,
        looping: true,
        field_3c: 0,
        hued: true,
    };
    let mut offsets = [DirectionOffset::default(); POSITION_OFFSET_DIRECTION_COUNT];
    offsets[1] = DirectionOffset { x: i8::MIN, y: i8::MAX };
    vec![
        EffectDefinition::Particle(particle.clone()),
        EffectDefinition::Particle(ParticleEffect { offset_y: 0.0, scale: 1.0, attach_mode: 1, duration: 0.0, looping: false, hued: false, ..particle }),
        EffectDefinition::Sound(SoundEffect { sound_id: SOUND_ID, value_08: f32::NAN }),
        EffectDefinition::RoundRobin(RoundRobinEffect { children: vec![sound(1), sound(2)] }),
        EffectDefinition::PositionOffset(PositionOffsetEffect { offsets }),
        EffectDefinition::Wait(WaitEffect { children: vec![sound(3)], min_delay_ms: 500, max_delay_ms: 2000, aligned: true }),
        EffectDefinition::Lod(LodEffect {
            levels: vec![
                EffectMapEntry { key: 1, children: vec![sound(4)] },
                EffectMapEntry { key: PARTICLE_LOD_MAX, children: vec![] },
            ],
        }),
        EffectDefinition::PropertyFlag(PropertyFlagEffect {
            mode: PropertyFlagMode::Property,
            mask: FLAG_MASK,
            property: HUE_PROPERTY,
            default_children: vec![sound(5)],
            cases: vec![EffectMapEntry { key: CASE_KEY, children: vec![sound(6)] }],
        }),
        EffectDefinition::PropertyFlag(PropertyFlagEffect {
            mode: PropertyFlagMode::FlagMask,
            mask: 0,
            property: 0,
            default_children: vec![],
            cases: vec![],
        }),
        EffectDefinition::Earthquake(EarthquakeEffect { amplitude: 0.1, duration: f64::INFINITY, frequency: 12.0 }),
        EffectDefinition::Random(RandomEffect {
            entries: vec![
                RandomEffectEntry { weight: 33.3, children: vec![sound(7)] },
                RandomEffectEntry { weight: 66.7, children: vec![] },
            ],
        }),
        EffectDefinition::Random(RandomEffect { entries: vec![] }),
        EffectDefinition::UserPreference(UserPreferenceEffect { mask: USER_PREFERENCE_FOOTSTEPS, children: vec![sound(8)] }),
    ]
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::enhanced::effect_definition::{parse_effect_definition_collection, read_effect_definitions};

    /// Equality by encoded bytes, so NaN fields compare by their bits.
    fn assert_same(left: &[EffectDefinition], right: &[EffectDefinition]) {
        assert_eq!(encode_effect_definitions(left), encode_effect_definitions(right));
    }

    #[test]
    fn collection_payload_round_trips() {
        let definitions = every_definition_type();
        let payload = encode_effect_definition_collection(&definitions);
        let parsed = parse_effect_definition_collection(&payload).unwrap();
        assert_same(&parsed, &definitions);
        assert_eq!(encode_effect_definition_collection(&parsed), payload);
    }

    #[test]
    fn tileart_tail_has_a_u8_count() {
        let definitions = every_definition_type();
        let tail = encode_tileart_effects(&definitions).unwrap();
        assert_eq!(usize::from(tail[0]), definitions.len());
        let parsed = read_effect_definitions(&mut Cursor::new(&tail[1..]), definitions.len()).unwrap();
        assert_same(&parsed, &definitions);

        let too_many = vec![definitions[0].clone(); TILEART_EFFECT_COUNT_MAX + 1];
        assert!(encode_tileart_effects(&too_many).is_err());
    }

    #[test]
    fn kingdom_reborn_grammar_round_trips_and_differs_where_documented() {
        use crate::enhanced::effect_definition::{
            ec_effect_type_key_of_kr, kr_effect_type_key, parse_effect_definition_collection_dialect, ParticleEffect,
            PropertyFlagEffect, EFFECT_TYPE_POSITION_OFFSET, EFFECT_TYPE_ROUND_ROBIN, EFFECT_TYPE_USER_PREFERENCE,
        };
        const KR: ClientDialect = ClientDialect::KingdomReborn;
        /// Bytes of a KR particle body (EC 28).
        const KR_PARTICLE_BODY_SIZE: usize = 27;
        /// EC tile effect (no KR key), KR container (EC 6), KR death (EC 3, which neither parser supports), and the
        /// first key past the KR enum.
        const EC_TILE_EFFECT: u32 = 5;
        const KR_CONTAINER: u32 = 5;
        const KR_DEATH: u32 = 3;
        const KR_PAST_THE_ENUM: u32 = 17;

        assert_eq!(kr_effect_type_key(EFFECT_TYPE_ROUND_ROBIN), Some(EFFECT_TYPE_ROUND_ROBIN));
        assert_eq!(kr_effect_type_key(EC_TILE_EFFECT), None);
        assert_eq!(kr_effect_type_key(EFFECT_TYPE_POSITION_OFFSET), Some(EFFECT_TYPE_POSITION_OFFSET - 1));
        assert_eq!(kr_effect_type_key(EFFECT_TYPE_USER_PREFERENCE), Some(EFFECT_TYPE_USER_PREFERENCE - 1));
        assert_eq!(kr_effect_type_key(EFFECT_TYPE_USER_PREFERENCE + 1), None);
        assert_eq!(ec_effect_type_key_of_kr(KR_CONTAINER), Some(EC_TILE_EFFECT + 1));
        assert_eq!(ec_effect_type_key_of_kr(KR_PAST_THE_ENUM), None);

        // Every type round trips once the KR limits hold: no hued particle, u32 masks.
        let mut definitions = every_definition_type();
        assert!(encode_effect_definition_collection_dialect(&definitions, KR).is_err());
        for definition in &mut definitions {
            match definition {
                EffectDefinition::Particle(effect) => effect.hued = false,
                EffectDefinition::PropertyFlag(effect) => effect.mask &= u64::from(u32::MAX),
                _ => {}
            }
        }
        let payload = encode_effect_definition_collection_dialect(&definitions, KR).unwrap();
        let parsed = parse_effect_definition_collection_dialect(&payload, KR).unwrap();
        assert_same(&parsed, &definitions);

        let particle = EffectDefinition::Particle(ParticleEffect { field_3c: 7, ..match &definitions[0] {
            EffectDefinition::Particle(effect) => effect.clone(),
            _ => unreachable!("the first definition is a particle"),
        } });
        let bytes = encode_effect_definitions_dialect(&[particle.clone()], KR).unwrap();
        assert_eq!(bytes.len(), size_of::<u32>() + KR_PARTICLE_BODY_SIZE);
        assert_eq!(encode_effect_definitions(&[particle]).len(), bytes.len() + 1);
        let flag = EffectDefinition::PropertyFlag(PropertyFlagEffect {
            mode: super::super::PropertyFlagMode::FlagMask,
            mask: u64::from(u32::MAX) + 1,
            property: 0,
            default_children: vec![],
            cases: vec![],
        });
        assert!(encode_effect_definitions_dialect(&[flag], KR).is_err());
        let mut unknown = Vec::new();
        put_u32(&mut unknown, 1);
        put_u32(&mut unknown, KR_DEATH);
        assert!(parse_effect_definition_collection_dialect(&unknown, KR).is_err());
    }
}
