//! Reads the audio of real clients: the Enhanced Client `Audio.uop` (`UO_EC_CLIENT_DIR`), the Classic sounds,
//! `Sound.def`, `Music.def` and `Music/Digital/Config.txt` (`UO_CC_CLIENT_DIR`, and every directory of
//! `UO_CLASSIC_CLIENT_ROOTS`, separated like `PATH`), and the Kingdom Reborn `Audio.uop` (`UO_KR_CLIENT_DIR`).
//!
//! Every entry and slot is probed and decoded; the figures are asserted when the entry count identifies the
//! reference release (the 2024 clients), and printed otherwise. Run with `--ignored --release`.

use crate::common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use rayon::prelude::*;
use uocf::audio::decode::{decode, decode_classic_pcm};
use uocf::audio::{AudioContent, AudioFormat};
use uocf::classic::audio_def::{DefTranslationTable, MUSIC_DEF_FILE_NAME, SOUND_DEF_FILE_NAME};
use uocf::classic::music::MusicConfig;
use uocf::classic::sound::SoundMap;
use uocf::enhanced::audio::{AudioKind, AudioPackage, AudioTables, AUDIO_UOP_FILE_NAME, SOUND_TABLE_ENTRY};
use uocf::enhanced::audio_edit::{AudioImportOptions, AudioPackageEditor, NewAudio};
use uocf::enhanced::kr_audio::{KR_EFFECTS_TABLE_ENTRY, KR_NEW_SOUND_PARAMETERS};
use uocf::utils::path::find_file_ignoring_case;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
const CLASSIC_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";

const REFERENCE_EC_ENTRIES: usize = 3435;
const REFERENCE_EC_SOUND_ROWS: usize = 1668;
const REFERENCE_EC_MUSIC_ROWS: usize = 107;
/// Entries by content in the reference `Audio.uop`: MP3, WAV and text (the two tables and three file lists).
const REFERENCE_EC_MP3: usize = 1764;
const REFERENCE_EC_WAV: usize = 1666;
const REFERENCE_EC_TEXT: usize = 5;
const REFERENCE_CC_SLOTS: usize = 1655;
const REFERENCE_KR_ENTRIES: usize = 1553;
/// The table path of the sound added to Kingdom Reborn's `Audio.uop`, and its structure name.
const KR_NEW_SOUND_PATH: &str = "Sounds\\uocf\\added.mp3";
const KR_NEW_SOUND_STRUCTURE: &str = "uocf\\added";
/// The server id whose MP3 the added sound holds.
const KR_MP3_SOURCE_SERVER_ID: u32 = 0;
/// The sound rows of the reference `audio_sounds.csv` whose path names no entry (typos, `pirates_` names).
const REFERENCE_EC_SILENT_SOUNDS: [std::ops::RangeInclusive<u32>; 7] =
    [19..=19, 856..=856, 1469..=1469, 1471..=1471, 1628..=1633, 1636..=1640, 1658..=1660];
/// The ids of the reference Classic `Sound.def` lines: the one the client drops (`805 {331 0`), and the one that
/// applies (the other ids have a slot of their own).
const REFERENCE_CC_DROPPED_DEF_ID: u32 = 805;
const REFERENCE_CC_APPLYING_DEF_ID: u32 = 856;

fn env_dir(env: &str) -> Option<PathBuf> {
    std::env::var_os(env).map(PathBuf::from)
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR (and UO_CC_CLIENT_DIR to compare the legacy sounds)"]
fn enhanced_audio_uop_entries_probe_and_decode() {
    let dir = env_dir(EC_CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {EC_CLIENT_DIR_ENV}"));
    let audio = AudioPackage::load(&dir).unwrap().unwrap_or_else(|| panic!("{EC_CLIENT_DIR_ENV}: no {AUDIO_UOP_FILE_NAME}"));
    let AudioTables::Enhanced { sounds, music } = audio.tables() else { panic!("not the Enhanced Client tables") };
    let hashes: Vec<u64> = audio.hashes().collect();
    println!("{} entries, {} sound rows, {} music rows, {} named by the tables", hashes.len(), sounds.len(), music.len(), audio.known_names().len());

    let results: Vec<(AudioFormat, Result<(), String>)> = hashes
        .par_iter()
        .map(|&hash| {
            let entry = audio.entry_by_hash(hash).unwrap().unwrap();
            let format = entry.content.format();
            let decoded = if format.is_decodable() {
                decode(&entry.payload, &entry.content).map(|_| ()).map_err(|error| format!("{hash:016X}: {error}"))
            } else {
                Ok(())
            };
            (format, decoded)
        })
        .collect();
    let mut formats: BTreeMap<AudioFormat, usize> = BTreeMap::new();
    for (format, _) in &results {
        *formats.entry(*format).or_default() += 1;
    }
    let failures: Vec<&String> = results.iter().filter_map(|(_, result)| result.as_ref().err()).collect();
    println!("formats {formats:?}; decode failures {failures:?}");
    assert!(failures.is_empty());

    let mut sound_kinds = BTreeMap::new();
    for kind in [AudioKind::Sound, AudioKind::LegacySound] {
        let found = sounds.keys().filter(|&&id| audio.entry_for(kind, id).unwrap().is_some()).count();
        sound_kinds.insert(format!("{kind:?}"), found);
    }
    let music_found = music.keys().filter(|&&id| audio.entry_for(AudioKind::Music, id).unwrap().is_some()).count();
    println!("ids with an entry: {sound_kinds:?}, music {music_found}");
    let silent: Vec<(AudioKind, u32)> = audio.silent_rows().iter().map(|row| (row.kind, row.id)).collect();
    println!("silent rows: {silent:?}");

    if let Some(cc) = env_dir(CC_CLIENT_DIR_ENV) {
        let classic = SoundMap::load(&cc).unwrap();
        let matches = sounds
            .keys()
            .filter(|&&id| {
                let (Some(entry), Some(slot)) = (audio.entry_for(AudioKind::LegacySound, id).unwrap(), classic.read_slot(id).unwrap())
                else {
                    return false;
                };
                let AudioContent::Wav(wav) = &entry.content else { return false };
                wav.data(&entry.payload) == Some(&slot.pcm_data[..])
            })
            .count();
        println!("legacy WAVs with the Classic slot's PCM: {matches}");
        if hashes.len() == REFERENCE_EC_ENTRIES {
            // Every slot of the reference Classic client.
            assert_eq!(matches, REFERENCE_CC_SLOTS);
        }
    }

    if hashes.len() == REFERENCE_EC_ENTRIES {
        let expected: Vec<(AudioKind, u32)> = REFERENCE_EC_SILENT_SOUNDS.iter().cloned().flatten().map(|id| (AudioKind::Sound, id)).collect();
        assert_eq!(silent, expected);
        assert_eq!((sounds.len(), music.len()), (REFERENCE_EC_SOUND_ROWS, REFERENCE_EC_MUSIC_ROWS));
        assert_eq!(formats.get(&AudioFormat::Mpeg(uocf::audio::mp3::MpegLayer::L3)), Some(&REFERENCE_EC_MP3));
        assert_eq!(formats.get(&AudioFormat::Wav), Some(&REFERENCE_EC_WAV));
        assert_eq!(formats.get(&AudioFormat::Text), Some(&REFERENCE_EC_TEXT));
    }
}

/// The Classic client directories: `UO_CC_CLIENT_DIR` and those of `UO_CLASSIC_CLIENT_ROOTS`.
fn classic_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = env_dir(CC_CLIENT_DIR_ENV).into_iter().collect();
    if let Some(roots) = std::env::var_os(CLASSIC_ROOTS_ENV) {
        dirs.extend(std::env::split_paths(&roots));
    }
    assert!(!dirs.is_empty(), "set {CC_CLIENT_DIR_ENV} and/or {CLASSIC_ROOTS_ENV}");
    dirs
}

#[test]
#[ignore = "needs a real Classic client: set UO_CC_CLIENT_DIR and/or UO_CLASSIC_CLIENT_ROOTS"]
fn classic_sounds_defs_and_music_parse() {
    for dir in classic_dirs() {
        let sounds = SoundMap::load(&dir).unwrap();
        let slots: Vec<u32> = (0..sounds.slot_count() as u32).collect();
        let read: Vec<_> = slots.par_iter().filter_map(|&slot| sounds.read_slot(slot).unwrap()).collect();
        let samples: u64 = read.iter().map(|sound| decode_classic_pcm(&sound.pcm_data).frames()).sum();
        let odd = read.iter().filter(|sound| sound.pcm_data.len() % 2 == 1).count();
        println!(
            "{}: {} slots of {} from {}, {samples} samples, {odd} with an odd PCM length, {} Sound.def ids",
            dir.display(),
            read.len(),
            sounds.slot_count(),
            sounds.source_label(),
            sounds.translation_count()
        );
        for name in [SOUND_DEF_FILE_NAME, MUSIC_DEF_FILE_NAME] {
            if let Some(path) = find_file_ignoring_case(&dir, name) {
                let table = DefTranslationTable::load(&path).unwrap().unwrap();
                let text = std::fs::read_to_string(&path).unwrap();
                assert_eq!(table.to_text(), text, "{name} is written back byte for byte");
                println!("  {name}: {} ids; {:?}", table.len(), table.client_notes());
                if let Some(dropped) = table.get(REFERENCE_CC_DROPPED_DEF_ID).filter(|_| name == SOUND_DEF_FILE_NAME) {
                    assert!(dropped.client_ignored.is_some() && dropped.client_targets().is_none(), "the client drops `805 {{331 0`");
                }
                assert!(table.iter().all(|entry| entry.extra.is_none_or(|extra| extra == 0)), "{name}: a third field other than 0");
            }
        }
        if let Some(config) = MusicConfig::load(&dir).unwrap() {
            let missing: Vec<_> = config.iter().filter(|entry| config.file_path(entry).is_none()).map(|entry| entry.file_name()).collect();
            let not_looping: Vec<_> = config.iter().filter(|entry| !entry.loops()).map(|entry| entry.id).collect();
            println!("  Config.txt: {} ids, missing files {missing:?}, not looping {not_looping:?}; {:?}", config.len(), config.client_notes());
            assert!(missing.is_empty());
        }
        let notes = sounds.translation_notes().unwrap();
        println!("  {notes:?}");
        if sounds.source_label() == uocf::classic::sound::SOUND_UOP_FILE_NAME && read.len() == REFERENCE_CC_SLOTS {
            assert_eq!(odd, 0);
            assert!(notes[0].contains(&format!("1 apply (ids [{REFERENCE_CC_APPLYING_DEF_ID}])")), "{notes:?}");
        }
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR"]
fn kingdom_reborn_audio_uop_entries_probe() {
    let path = common::kr_client_file(AUDIO_UOP_FILE_NAME);
    let audio = AudioPackage::load_file(&path).unwrap();
    let AudioTables::KingdomReborn(kr) = audio.tables() else { panic!("not Kingdom Reborn's tables") };
    for (entry, text) in [
        (uocf::enhanced::audio::SOUND_TABLE_ENTRY, kr.sounds.to_text()),
        (uocf::enhanced::kr_audio::KR_EFFECTS_TABLE_ENTRY, kr.effects.as_ref().unwrap().to_text()),
    ] {
        let shipped = audio.entry_by_name(entry).unwrap().unwrap().payload;
        assert!(shipped[..] == *text.as_bytes(), "{entry} is written back byte for byte");
    }
    let effects = kr.effects.as_ref().unwrap();
    let server_ids: Vec<u32> = effects.rows().filter_map(|row| row.server_id()).collect();
    let playable = server_ids.iter().filter(|&&id| audio.entry_for(AudioKind::Sound, id).unwrap().is_some()).count();
    let notes = audio.table_warnings();
    println!(
        "{} sound structures, {} effects rows, {playable} server ids reach an entry; {} notes: {notes:#?}",
        kr.sounds.sounds().count(),
        server_ids.len(),
        notes.len()
    );
    let hashes: Vec<u64> = audio.hashes().collect();
    let mut formats: BTreeMap<AudioFormat, usize> = BTreeMap::new();
    for &hash in &hashes {
        *formats.entry(audio.entry_by_hash(hash).unwrap().unwrap().content.format()).or_default() += 1;
    }
    println!("{} entries: {formats:?}", hashes.len());
    if hashes.len() == REFERENCE_KR_ENTRIES {
        assert_eq!(formats.get(&AudioFormat::Unknown), None);
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR"]
fn kingdom_reborn_audio_add_plays_a_new_server_id() {
    let path = common::kr_client_file(AUDIO_UOP_FILE_NAME);
    let out = common::temp_dir("audio_client_files_kr_add");
    let shipped = AudioPackage::load_file(&path).unwrap();
    let AudioTables::KingdomReborn(kr) = shipped.tables() else { panic!("not Kingdom Reborn's tables") };
    let effects = kr.effects.as_ref().unwrap();
    let server_id = effects.rows().filter_map(|row| row.server_id()).max().unwrap() + 1;
    let mp3 = shipped.entry_for(AudioKind::Sound, KR_MP3_SOURCE_SERVER_ID).unwrap().unwrap().payload;

    let mut editor = AudioPackageEditor::load(&path).unwrap();
    let new = NewAudio::Id { kind: AudioKind::Sound, id: server_id, path: Some(KR_NEW_SOUND_PATH.to_owned()), loops: None };
    let added = editor.add(&new, &mp3, AudioImportOptions::default()).unwrap();
    assert_eq!(added.tables_with_new_rows, vec![SOUND_TABLE_ENTRY, KR_EFFECTS_TABLE_ENTRY]);
    let output = out.join(AUDIO_UOP_FILE_NAME);
    editor.save_as(&output).unwrap();

    let edited = AudioPackage::load_file(&output).unwrap();
    let AudioTables::KingdomReborn(after) = edited.tables() else { panic!("not Kingdom Reborn's tables") };
    // Each table is the shipped one, byte for byte, and one line.
    let new_lines = [
        (SOUND_TABLE_ENTRY, format!(",,{KR_NEW_SOUND_STRUCTURE},{KR_NEW_SOUND_PATH},{KR_NEW_SOUND_PARAMETERS}\r\n")),
        (KR_EFFECTS_TABLE_ENTRY, format!(",{},{KR_NEW_SOUND_STRUCTURE}\r\n", server_id + 1)),
    ];
    for (entry, line) in new_lines {
        let before = shipped.entry_by_name(entry).unwrap().unwrap().payload;
        let now = edited.entry_by_name(entry).unwrap().unwrap().payload;
        assert_eq!(now[..], [&before[..], line.as_bytes()].concat()[..], "{entry}");
    }
    assert_eq!(edited.entry_for(AudioKind::Sound, server_id).unwrap().unwrap().payload, mp3, "the new server id plays the MP3");
    assert_eq!(after.sounds.sounds().count(), kr.sounds.sounds().count() + 1);
    assert_eq!(edited.hashes().count(), shipped.hashes().count() + 1);
    for row in effects.rows() {
        let Some(id) = row.server_id() else { continue };
        let name = |audio: &AudioPackage| audio.entry_name_for(AudioKind::Sound, id);
        assert_eq!(name(&edited), name(&shipped), "server id {id} plays what it played");
    }
    assert_eq!(edited.table_warnings(), shipped.table_warnings());
    println!("server id {server_id} (ID# {}) plays {}", server_id + 1, added.name);
    let _ = std::fs::remove_dir_all(&out);
}
