//! Serializers of effect definitions: the exact inverse of the readers (flags are written as 0 or 1).

use color_eyre::eyre::{self, Result};

use super::{EffectDefinition, EffectMapEntry};

/// Largest definition count of a tileart record, whose count is a u8.
pub const TILEART_EFFECT_COUNT_MAX: usize = u8::MAX as usize;

/// Appends one definition: its u32 type key, then its body.
pub fn write_effect_definition(out: &mut Vec<u8>, definition: &EffectDefinition) {
    put_u32(out, definition.type_key());
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
            out.push(u8::from(effect.hued));
        }
        EffectDefinition::Sound(effect) => {
            put_u32(out, effect.sound_id);
            put_f32(out, effect.value_08);
        }
        EffectDefinition::RoundRobin(effect) => write_effect_list(out, &effect.children),
        EffectDefinition::PositionOffset(effect) => {
            for offset in &effect.offsets {
                out.extend_from_slice(&offset.x.to_le_bytes());
                out.extend_from_slice(&offset.y.to_le_bytes());
            }
        }
        EffectDefinition::Wait(effect) => {
            write_effect_list(out, &effect.children);
            put_u32(out, effect.min_delay_ms);
            put_u32(out, effect.max_delay_ms);
            out.push(u8::from(effect.aligned));
        }
        EffectDefinition::Lod(effect) => write_map(out, &effect.levels),
        EffectDefinition::PropertyFlag(effect) => {
            out.push(effect.mode as u8);
            out.extend_from_slice(&effect.mask.to_le_bytes());
            put_u32(out, effect.property);
            write_effect_list(out, &effect.default_children);
            write_map(out, &effect.cases);
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
                write_effect_list(out, &entry.children);
            }
        }
        EffectDefinition::UserPreference(effect) => {
            put_u32(out, effect.mask);
            write_effect_list(out, &effect.children);
        }
    }
}

/// The definitions one after the other, without a count (the inverse of `read_effect_definitions`).
pub fn encode_effect_definitions(definitions: &[EffectDefinition]) -> Vec<u8> {
    let mut out = Vec::new();
    for definition in definitions {
        write_effect_definition(&mut out, definition);
    }
    out
}

/// Appends an effect list: u32 count, then the definitions.
pub fn write_effect_list(out: &mut Vec<u8>, definitions: &[EffectDefinition]) {
    put_count(out, definitions.len());
    for definition in definitions {
        write_effect_definition(out, definition);
    }
}

/// The payload of one `EffectDefinitionCollection.uop` entry, the inverse of
/// `parse_effect_definition_collection`.
pub fn encode_effect_definition_collection(definitions: &[EffectDefinition]) -> Vec<u8> {
    let mut out = Vec::new();
    write_effect_list(&mut out, definitions);
    out
}

/// The effect tail of a `tileart.uop` record: u8 count, then the definitions. Fails above
/// [`TILEART_EFFECT_COUNT_MAX`] definitions.
pub fn encode_tileart_effects(definitions: &[EffectDefinition]) -> Result<Vec<u8>> {
    let Ok(count) = u8::try_from(definitions.len()) else {
        eyre::bail!(
            "{} effect definitions do not fit a tileart record (at most {TILEART_EFFECT_COUNT_MAX})",
            definitions.len()
        );
    };
    let mut out = vec![count];
    out.extend(encode_effect_definitions(definitions));
    Ok(out)
}

fn write_map(out: &mut Vec<u8>, entries: &[EffectMapEntry]) {
    put_count(out, entries.len());
    for entry in entries {
        put_u32(out, entry.key);
        write_effect_list(out, &entry.children);
    }
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
}
