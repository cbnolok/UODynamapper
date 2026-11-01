//! Writes the audio of real clients back: the Enhanced Client and Kingdom Reborn `Audio.uop` (`UO_EC_CLIENT_DIR`,
//! `UO_KR_CLIENT_DIR`), and the Classic sounds, `Sound.def` and `Music.def` (`UO_CC_CLIENT_DIR`, and every
//! directory of `UO_CLASSIC_CLIENT_ROOTS`, separated like `PATH`). Unchanged files must come back byte for byte;
//! edits must leave every other entry or slot as it was. Copies go under the temporary directory (`TMPDIR`).

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use uocf::audio::validate::{enhanced_format_report, FormatPolicy};
use uocf::audio::{AudioContent, AudioFormat};
use uocf::classic::audio_def::{DefTranslationTable, MUSIC_DEF_FILE_NAME, SOUND_DEF_FILE_NAME};
use uocf::classic::sound::{SoundMap, MAX_SOUND_ID, SOUND_HEADER_BYTES};
use uocf::classic::sound_edit::{ClassicSoundEditor, ClassicSoundWrite, SoundMulFiles};
use uocf::enhanced::audio::{
    write_table, AudioKind, AudioPackage, AudioTables, AUDIO_UOP_FILE_NAME, MUSIC_TABLE_ENTRY, MUSIC_TABLE_HEADER,
    SOUND_TABLE_ENTRY, SOUND_TABLE_HEADER,
};
use uocf::enhanced::audio_edit::{AudioImportOptions, AudioPackageEditor, AudioTarget, AudioWrite, NewAudio};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
const CLASSIC_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// An id past every shipped sound and music row, for the added rows.
const NEW_ID: u32 = 60_000;
const NEW_SOUND_PATH: &str = "Sounds\\uocf_test\\added.mp3";
const NEW_MUSIC_PATH: &str = "Music/uocf_test_added.mp3";
/// The index entries a truncated `soundidx.mul` copy keeps, and how far past its end a slot is added: the index
/// grows to it with empty entries.
const TRUNCATED_INDEX_ENTRIES: u32 = 1000;
const GROWN_SLOT: u32 = 1010;

fn env_dir(env: &str) -> Option<PathBuf> {
    std::env::var_os(env).map(PathBuf::from)
}

/// The unpacked payload of every entry (not the unused slots of the last block), by hash.
fn payloads(package: &UopPackage) -> BTreeMap<u64, Vec<u8>> {
    let hashes: Vec<u64> = package.iter_files().filter(|file| !file.is_empty_slot()).map(|file| file.filename_hash()).collect();
    hashes.par_iter().map(|&hash| (hash, package.unpack_file_by_hash(hash).unwrap().unwrap())).collect()
}

/// Loads `path`, saves it unchanged to `output`, and checks every entry's payload; returns whether the whole file
/// came back byte for byte.
fn round_trip_audio_uop(path: &Path, output: &Path) -> bool {
    let mut editor = AudioPackageEditor::load(path).unwrap();
    editor.save_as(output).unwrap();
    let (original, copy) = (UopPackage::load(path).unwrap(), UopPackage::load(output).unwrap());
    let (before, after) = (payloads(&original), payloads(&copy));
    assert_eq!(before.len(), after.len());
    let differing = before.iter().filter(|(hash, payload)| after.get(hash) != Some(payload)).count();
    assert_eq!(differing, 0, "{}: entries whose payload changed", path.display());
    let identical = std::fs::read(path).unwrap() == std::fs::read(output).unwrap();
    println!("{}: {} entry payloads identical; whole file identical: {identical}", path.display(), before.len());
    identical
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR"]
fn enhanced_audio_uop_writes_back_and_edits() {
    let dir = env_dir(EC_CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {EC_CLIENT_DIR_ENV}"));
    let path = find_file_ignoring_case(&dir, AUDIO_UOP_FILE_NAME).unwrap_or_else(|| panic!("no {AUDIO_UOP_FILE_NAME}"));
    let out = common::temp_dir("audio_writers_ec");

    assert!(round_trip_audio_uop(&path, &out.join("Audio.unchanged.uop")), "an unchanged Audio.uop is written back byte for byte");

    // The tables come back from their rows byte for byte, and every shipped file passes the format checks.
    let audio = AudioPackage::load_file(&path).unwrap();
    let AudioTables::Enhanced { sounds, music } = audio.tables() else { panic!("not the Enhanced Client tables") };
    for (entry, header, rows, with_loop) in [(SOUND_TABLE_ENTRY, SOUND_TABLE_HEADER, sounds, false), (MUSIC_TABLE_ENTRY, MUSIC_TABLE_HEADER, music, true)] {
        let shipped = audio.entry_by_name(entry).unwrap().unwrap().payload;
        assert_eq!(write_table(header, rows, with_loop).as_bytes(), &shipped[..], "{entry}");
    }
    let hashes: Vec<u64> = audio.hashes().collect();
    let failing: Vec<String> = hashes
        .par_iter()
        .filter_map(|&hash| {
            let entry = audio.entry_by_hash(hash).unwrap().unwrap();
            if entry.content == AudioContent::Text {
                return None;
            }
            let name = entry.name.clone().unwrap_or_else(|| match entry.content.format() {
                AudioFormat::Wav => "unnamed.wav".to_owned(),
                _ => "unnamed.mp3".to_owned(),
            });
            let report = enhanced_format_report(&name, &entry.content);
            (!report.is_verified()).then(|| format!("{name} ({hash:016X}): {report:?}"))
        })
        .collect();
    println!("{} shipped entries fail the format checks: {failing:?}", failing.len());
    assert!(failing.is_empty());

    // Replace a legacy WAV and an MP3 with other shipped files, add a sound and a music id, and check the rest.
    let options = AudioImportOptions { policy: FormatPolicy::VerifiedOnly, convert: false };
    let first_sound = *sounds.keys().next().unwrap();
    let other_sound = sounds.keys().copied().find(|&id| id != first_sound && audio.entry_for(AudioKind::LegacySound, id).unwrap().is_some()).unwrap();
    let mp3 = audio.entry_for(AudioKind::Sound, other_sound).unwrap().unwrap().payload;
    let wav = audio.entry_for(AudioKind::LegacySound, other_sound).unwrap().unwrap().payload;
    let mut editor = AudioPackageEditor::load(&path).unwrap();
    let replaced_mp3 = editor.replace(&AudioTarget::Id(AudioKind::Sound, first_sound), &mp3, options).unwrap();
    let replaced_wav = editor.replace(&AudioTarget::Id(AudioKind::LegacySound, first_sound), &wav, options).unwrap();
    let added_sound = NewAudio::Id { kind: AudioKind::Sound, id: NEW_ID, path: Some(NEW_SOUND_PATH.to_owned()), loops: None };
    let added = editor.add(&added_sound, &mp3, options).unwrap();
    let added_wav = editor.add(&NewAudio::Id { kind: AudioKind::LegacySound, id: NEW_ID, path: None, loops: None }, &wav, options).unwrap();
    let added_music = NewAudio::Id { kind: AudioKind::Music, id: NEW_ID, path: Some(NEW_MUSIC_PATH.to_owned()), loops: Some(true) };
    editor.add(&added_music, &mp3, options).unwrap();
    for put in [&replaced_mp3, &replaced_wav, &added, &added_wav] {
        println!("{:?} {} ({:?})", put.write, put.name, put.compression);
    }
    assert_eq!((replaced_mp3.write, replaced_wav.write, added.write), (AudioWrite::Replaced, AudioWrite::Replaced, AudioWrite::Added));
    let edited_path = out.join("Audio.edited.uop");
    editor.save_as(&edited_path).unwrap();

    let edited = AudioPackage::load_file(&edited_path).unwrap();
    assert_eq!(edited.entry_for(AudioKind::Sound, first_sound).unwrap().unwrap().payload, mp3);
    assert_eq!(edited.entry_for(AudioKind::LegacySound, first_sound).unwrap().unwrap().payload, wav);
    assert_eq!(edited.entry_for(AudioKind::Sound, NEW_ID).unwrap().unwrap().payload, mp3);
    assert_eq!(edited.entry_for(AudioKind::LegacySound, NEW_ID).unwrap().unwrap().payload, wav);
    assert_eq!(edited.entry_for(AudioKind::Music, NEW_ID).unwrap().unwrap().payload, mp3);
    assert_eq!(edited.music_row(NEW_ID).unwrap().loops, Some(true));
    let (before, after) = (payloads(audio.package()), payloads(edited.package()));
    let touched = [replaced_mp3.hash, replaced_wav.hash, hash_file_name_single(SOUND_TABLE_ENTRY), hash_file_name_single(MUSIC_TABLE_ENTRY)];
    let changed = before.iter().filter(|(hash, payload)| !touched.contains(hash) && after.get(hash) != Some(payload)).count();
    assert_eq!(changed, 0, "untouched entries changed");
    assert_eq!(after.len(), before.len() + 3);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR"]
fn kingdom_reborn_audio_uop_writes_back() {
    let path = common::kr_client_file(AUDIO_UOP_FILE_NAME);
    let out = common::temp_dir("audio_writers_kr");
    let identical = round_trip_audio_uop(&path, &out.join("Audio.unchanged.uop"));
    assert!(identical, "an unchanged Kingdom Reborn Audio.uop is written back byte for byte");
    let _ = std::fs::remove_dir_all(&out);
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
fn classic_sounds_and_defs_write_back_and_edit() {
    for (index, dir) in classic_dirs().into_iter().enumerate() {
        let out = common::temp_dir(&format!("audio_writers_classic_{index}"));
        for name in [SOUND_DEF_FILE_NAME, MUSIC_DEF_FILE_NAME] {
            if let Some(path) = find_file_ignoring_case(&dir, name) {
                let table = DefTranslationTable::load(&path).unwrap().unwrap();
                assert_eq!(table.to_text().as_bytes(), &std::fs::read(&path).unwrap()[..], "{}", path.display());
                println!("{}: written back byte for byte", path.display());
            }
        }

        let unchanged = out.join("unchanged");
        let mut editor = ClassicSoundEditor::load(&dir).unwrap();
        let names: Vec<String> = editor.file_names().into_iter().map(str::to_owned).collect();
        editor.save_to(&unchanged).unwrap();
        for name in &names {
            let identical = std::fs::read(dir.join(name)).unwrap() == std::fs::read(unchanged.join(name)).unwrap();
            println!("{}: {name} unchanged, written back byte for byte: {identical}", dir.display());
            assert!(identical, "{}: {name}", dir.display());
        }

        // Replace the first slot with the second's PCM, keeping its header; add a sound in the first empty slot
        // the client plays (the MUL clients' index has 4096 entries, past the highest id it plays).
        let original = SoundMap::load(&dir).unwrap();
        let slots: Vec<u32> = (0..original.slot_count() as u32).filter(|&slot| original.read_slot(slot).unwrap().is_some()).collect();
        let (first, second) = (slots[0], slots[1]);
        let second_wav = original.read_slot(second).unwrap().unwrap().wav_bytes();
        let first_sound = original.read_slot(first).unwrap().unwrap();
        assert_eq!(editor.put_sound(first, &first_sound.wav_bytes(), None, false).unwrap().write, ClassicSoundWrite::Unchanged);
        let replaced = editor.put_sound(first, &second_wav, None, false).unwrap();
        let new_slot = (0..=MAX_SOUND_ID).find(|&slot| original.read_slot(slot).unwrap().is_none()).expect("an empty slot");
        let added = editor.put_sound(new_slot, &second_wav, Some("uocf_test.wav"), false).unwrap();
        assert_eq!((replaced.write, added.write), (ClassicSoundWrite::Replaced, ClassicSoundWrite::Added));
        let edited_dir = out.join("edited");
        editor.save_to(&edited_dir).unwrap();

        let edited = SoundMap::load(&edited_dir).unwrap();
        let replaced_slot = edited.read_slot(first).unwrap().unwrap();
        assert_eq!(replaced_slot.header, first_sound.header, "the replaced slot keeps its header");
        assert_eq!(replaced_slot.pcm_data, original.read_slot(second).unwrap().unwrap().pcm_data);
        assert_eq!(edited.read_slot(new_slot).unwrap().unwrap().name, "uocf_test.wav");
        let untouched = slots.par_iter().filter(|&&slot| slot != first).all(|&slot| {
            edited.read_slot_bytes(slot).unwrap() == original.read_slot_bytes(slot).unwrap()
        });
        assert!(untouched, "{}: untouched slots changed", dir.display());
        if let [idx, mul] = &names[..] {
            let files = SoundMulFiles::load(&edited_dir.join(idx), &edited_dir.join(mul)).unwrap();
            assert_eq!(files.slot_count(), original.slot_count().max(new_slot as usize + 1));
            assert_eq!(files.slot(new_slot).unwrap().len(), SOUND_HEADER_BYTES + replaced_slot.pcm_data.len());

            // A truncated index copy grows to a slot past its end, with empty entries as the shipped ones.
            let truncated = out.join("truncated");
            std::fs::create_dir_all(&truncated).unwrap();
            let idx_bytes = std::fs::read(dir.join(idx)).unwrap();
            let kept = TRUNCATED_INDEX_ENTRIES as usize * uocf::classic::sound_edit::SOUND_INDEX_ENTRY_BYTES;
            std::fs::write(truncated.join(idx), &idx_bytes[..kept]).unwrap();
            std::fs::copy(dir.join(mul), truncated.join(mul)).unwrap();
            let mut grower = ClassicSoundEditor::load(&truncated).unwrap();
            let put = grower.put_sound(GROWN_SLOT, &second_wav, Some("uocf_grown.wav"), false).unwrap();
            assert_eq!(put.write, ClassicSoundWrite::Added);
            assert!(put.warnings.iter().any(|warning| warning.contains("extends the index")), "{:?}", put.warnings);
            let grown_dir = out.join("grown");
            grower.save_to(&grown_dir).unwrap();
            let grown = SoundMulFiles::load(&grown_dir.join(idx), &grown_dir.join(mul)).unwrap();
            let before = SoundMulFiles::load(&truncated.join(idx), &truncated.join(mul)).unwrap();
            assert_eq!(grown.slot_count(), GROWN_SLOT as usize + 1);
            for slot in TRUNCATED_INDEX_ENTRIES..GROWN_SLOT {
                let entry = grown.entry(slot).unwrap();
                assert!(entry.is_empty() && entry.extra == slot + 1, "slot {slot}: {entry:?}");
            }
            assert!((0..TRUNCATED_INDEX_ENTRIES).all(|slot| grown.slot(slot) == before.slot(slot) && grown.entry(slot).unwrap().extra == before.entry(slot).unwrap().extra));
            assert_eq!(SoundMap::load(&grown_dir).unwrap().read_slot(GROWN_SLOT).unwrap().unwrap().name, "uocf_grown.wav");
            println!("{}: a {TRUNCATED_INDEX_ENTRIES}-entry index grew to slot {GROWN_SLOT}", dir.display());
        }
        println!("{}: slot {first} replaced, slot {new_slot} added, the other {} slots unchanged", dir.display(), slots.len() - 1);
        let _ = std::fs::remove_dir_all(&out);
    }
}
