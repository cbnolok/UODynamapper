//! Bounded PE32 inspection shared by the KR handshake resolvers.
//! Every read is checked against one file-backed section; writable globals may live in its virtual BSS tail.

use std::collections::BTreeSet;

use super::encryption::PATTERN_WILDCARD;
use super::pe::{PeImage, SECTION_EXECUTE};

const WORD_SIZE: usize = size_of::<u32>();
const REL32_SIZE: usize = 1 + WORD_SIZE;
const CALL_REL32: u8 = 0xe8;
const JMP_REL32: u8 = 0xe9;
const DOS_PE_OFFSET: usize = 0x3c;
const PE_SIGNATURE: &[u8] = b"PE\0\0";
const COFF_SIZE: usize = 20;
const COFF_OPTIONAL_SIZE_OFFSET: usize = 16;
const PE32_MAGIC: u16 = 0x10b;
const OPTIONAL_DIRECTORY_COUNT: usize = 92;
const OPTIONAL_DIRECTORIES: usize = 96;
const DIRECTORY_SIZE: usize = 8;
const IMPORT_DIRECTORY: usize = 1;
const IMPORT_DESCRIPTOR_SIZE: usize = 20;
const IMPORT_NAME_OFFSET: usize = 12;
const IMPORT_IAT_OFFSET: usize = 16;
const ORDINAL_FLAG: u32 = 0x8000_0000;
const ORDINAL_MASK: u32 = 0x0000_ffff;
const IMPORT_HINT_SIZE: u32 = 2;
const MAX_IMPORT_DESCRIPTORS: usize = 4096;
const MAX_IMPORT_THUNKS: usize = 65536;
const MAX_MODULE_NAME: usize = 256;
const MAX_SYMBOL_NAME: usize = 4096;
const COL_SIZE: usize = 20;
const COL_TYPE_OFFSET: u32 = 12;
const COL_HIERARCHY_OFFSET: u32 = 16;
const TYPE_NAME_OFFSET: u32 = 8;
const HIERARCHY_SIZE: usize = 16;
const HIERARCHY_COUNT_OFFSET: u32 = 8;
const HIERARCHY_BASES_OFFSET: u32 = 12;
const BASE_DESCRIPTOR_SIZE: usize = 24;
const MAX_BASE_CLASSES: u32 = 1024;

/// A generated replacement. `stock` is the actual input span, including an earlier replacement on upgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedHandshake {
    pub offset: usize,
    pub stock: Vec<u8>,
    pub patched: Vec<u8>,
    pub removed: bool,
    /// File offsets of every absolute DWORD operand in the new span, including unchanged operands.
    pub absolute_operands: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Import {
    pub module: String,
    pub name: Option<String>,
    pub ordinal: Option<u16>,
}

pub(super) struct Image<'a> {
    pub bytes: &'a [u8],
    pub pe: &'a PeImage,
}

fn raw_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(size_of::<u16>())?)?.try_into().ok()?))
}

fn raw_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(WORD_SIZE)?)?.try_into().ok()?))
}

impl<'a> Image<'a> {
    pub fn new(bytes: &'a [u8], pe: &'a PeImage) -> Self {
        Self { bytes, pe }
    }

    pub fn read(&self, va: u32, len: usize) -> Option<&'a [u8]> {
        let length = u32::try_from(len).ok()?;
        va.checked_add(length)?;
        let section = self.pe.sections.iter().find(|section| {
            va.checked_sub(section.virtual_address).is_some_and(|delta| {
                delta < section.raw_size && length <= section.raw_size - delta
            })
        })?;
        let delta = va.checked_sub(section.virtual_address)? as usize;
        let at = (section.raw_offset as usize).checked_add(delta)?;
        self.bytes.get(at..at.checked_add(len)?)
    }

    pub fn u32(&self, va: u32) -> Option<u32> {
        Some(u32::from_le_bytes(self.read(va, WORD_SIZE)?.try_into().ok()?))
    }

    pub fn is_code(&self, va: u32) -> bool {
        self.read(va, 1).is_some()
            && self.pe.section_of_va(va).is_some_and(|section| section.characteristics & SECTION_EXECUTE != 0)
    }

    pub fn is_data(&self, va: u32, len: usize) -> bool {
        self.read(va, len).is_some()
            && self.pe.section_of_va(va).is_some_and(|section| section.is_data() && section.is_readable())
    }

    /// Validate a writable global's virtual storage without reading it (BSS has no file contents).
    pub fn is_writable_data(&self, va: u32, len: usize) -> bool {
        let Ok(length) = u32::try_from(len) else { return false };
        if va.checked_add(length.saturating_sub(1)).is_none() {
            return false;
        }
        self.pe.sections.iter().any(|section| {
            let extent = section.virtual_size.max(section.raw_size);
            section.is_readable() && section.is_writable() && !section.is_code()
                && section.virtual_address.checked_add(extent.saturating_sub(1)).is_some()
                && va.checked_sub(section.virtual_address).is_some_and(|delta| {
                    delta < extent && length <= extent - delta
                })
        })
    }

    /// Decode a direct rel32 edge without allowing address arithmetic to wrap through zero.
    pub fn rel_target(&self, va: u32, opcode: u8) -> Option<u32> {
        if !self.is_code(va) {
            return None;
        }
        let code = self.read(va, REL32_SIZE)?;
        if code[0] != opcode {
            return None;
        }
        let displacement = i32::from_le_bytes(code[1..].try_into().ok()?);
        let next = va.checked_add(REL32_SIZE as u32)?;
        let target = next.checked_add_signed(displacement)?;
        self.is_code(target).then_some(target)
    }

    pub fn call_target(&self, va: u32) -> Option<u32> {
        self.rel_target(va, CALL_REL32)
    }

    pub fn jump_target(&self, va: u32) -> Option<u32> {
        self.rel_target(va, JMP_REL32)
    }

    fn data_string(&self, va: u32, limit: usize) -> Option<&'a str> {
        for len in 1..=limit {
            if !self.is_data(va, len) {
                return None;
            }
            let value = self.read(va, len)?;
            if value[len - 1] == 0 {
                let text = std::str::from_utf8(&value[..len - 1]).ok()?;
                return (!text.is_empty() && text.is_ascii()).then_some(text);
            }
        }
        None
    }

    /// Validate x86 MSVC COL, type descriptor and hierarchy before returning the decorated class name.
    pub fn rtti_type(&self, vtable: u32) -> Option<&'a str> {
        if !self.is_data(vtable, WORD_SIZE) || !self.is_code(self.u32(vtable)?) {
            return None;
        }
        let locator_slot = vtable.checked_sub(WORD_SIZE as u32)?;
        if !self.is_data(locator_slot, WORD_SIZE) {
            return None;
        }
        let locator = self.u32(locator_slot)?;
        if !self.is_data(locator, COL_SIZE) || self.u32(locator)? != 0 {
            return None;
        }
        let descriptor = self.u32(locator.checked_add(COL_TYPE_OFFSET)?)?;
        if !self.is_data(descriptor, TYPE_NAME_OFFSET as usize) {
            return None;
        }
        let type_info_vtable = self.u32(descriptor)?;
        if !self.is_data(type_info_vtable, WORD_SIZE) || !self.is_code(self.u32(type_info_vtable)?) {
            return None;
        }
        let name = self.data_string(descriptor.checked_add(TYPE_NAME_OFFSET)?, MAX_SYMBOL_NAME)?;
        if !(name.starts_with(".?AV") || name.starts_with(".?AU")) || !name.ends_with("@@") {
            return None;
        }
        let hierarchy = self.u32(locator.checked_add(COL_HIERARCHY_OFFSET)?)?;
        if !self.is_data(hierarchy, HIERARCHY_SIZE) || self.u32(hierarchy)? != 0 {
            return None;
        }
        let count = self.u32(hierarchy.checked_add(HIERARCHY_COUNT_OFFSET)?)?;
        if count == 0 || count > MAX_BASE_CLASSES {
            return None;
        }
        let bases = self.u32(hierarchy.checked_add(HIERARCHY_BASES_OFFSET)?)?;
        let array_size = (count as usize).checked_mul(WORD_SIZE)?;
        if !self.is_data(bases, array_size) {
            return None;
        }
        for index in 0..count {
            let base = self.u32(bases.checked_add(index.checked_mul(WORD_SIZE as u32)?)?)?;
            if !self.is_data(base, BASE_DESCRIPTOR_SIZE) {
                return None;
            }
            let base_type = self.u32(base)?;
            if !self.is_data(base_type, TYPE_NAME_OFFSET as usize) || (index == 0 && base_type != descriptor) {
                return None;
            }
        }
        Some(name)
    }

    /// Find validated vtable references for an exact decorated name; the caller decides whether one is unique.
    pub fn vtables(&self, type_name: &str) -> Vec<u32> {
        let mut found = BTreeSet::new();
        for section in self.pe.sections.iter().filter(|section| section.is_data() && section.is_readable()) {
            let Ok(size) = usize::try_from(section.raw_size) else { continue };
            for offset in (0..size.saturating_sub(WORD_SIZE - 1)).step_by(WORD_SIZE) {
                let Some(va) = section.virtual_address.checked_add(offset as u32) else { continue };
                if self.rtti_type(va) == Some(type_name) {
                    found.insert(va);
                }
            }
        }
        found.into_iter().collect()
    }

    pub fn scan(&self, pattern: &[u16]) -> Vec<u32> {
        if pattern.is_empty() {
            return Vec::new();
        }
        let mut found = BTreeSet::new();
        for section in self.pe.sections.iter().filter(|section| section.is_code()) {
            let Some(code) = self.read(section.virtual_address, section.raw_size as usize) else { continue };
            for (at, window) in code.windows(pattern.len()).enumerate() {
                if window.iter().zip(pattern).all(|(byte, expected)| *expected == PATTERN_WILDCARD || u16::from(*byte) == *expected)
                    && let Some(va) = section.virtual_address.checked_add(at as u32)
                {
                    found.insert(va);
                }
            }
        }
        found.into_iter().collect()
    }

    fn import_directory(&self) -> Option<(u32, usize)> {
        let nt = raw_u32(self.bytes, DOS_PE_OFFSET)? as usize;
        if self.bytes.get(nt..nt.checked_add(PE_SIGNATURE.len())?)? != PE_SIGNATURE {
            return None;
        }
        let coff = nt.checked_add(PE_SIGNATURE.len())?;
        let optional_size = usize::from(raw_u16(self.bytes, coff.checked_add(COFF_OPTIONAL_SIZE_OFFSET)?)?);
        let optional = coff.checked_add(COFF_SIZE)?;
        let header = self.bytes.get(optional..optional.checked_add(optional_size)?)?;
        if raw_u16(header, 0)? != PE32_MAGIC || raw_u32(header, OPTIONAL_DIRECTORY_COUNT)? <= IMPORT_DIRECTORY as u32 {
            return None;
        }
        let entry = OPTIONAL_DIRECTORIES.checked_add(IMPORT_DIRECTORY.checked_mul(DIRECTORY_SIZE)?)?;
        let rva = raw_u32(header, entry)?;
        let size = raw_u32(header, entry.checked_add(WORD_SIZE)?)? as usize;
        if rva == 0 || size < IMPORT_DESCRIPTOR_SIZE {
            return None;
        }
        let va = self.pe.image_base.checked_add(rva)?;
        self.is_data(va, size).then_some((va, size))
    }

    fn imports(&self) -> Option<Vec<(u32, Import)>> {
        let (directory, size) = self.import_directory()?;
        let mut found = Vec::new();
        let descriptors = (size / IMPORT_DESCRIPTOR_SIZE).min(MAX_IMPORT_DESCRIPTORS);
        for index in 0..descriptors {
            let va = directory.checked_add(u32::try_from(index.checked_mul(IMPORT_DESCRIPTOR_SIZE)?).ok()?)?;
            let descriptor = self.read(va, IMPORT_DESCRIPTOR_SIZE)?;
            if descriptor.iter().all(|byte| *byte == 0) {
                return Some(found);
            }
            let original = raw_u32(descriptor, 0)?;
            let module_rva = raw_u32(descriptor, IMPORT_NAME_OFFSET)?;
            let iat_rva = raw_u32(descriptor, IMPORT_IAT_OFFSET)?;
            if module_rva == 0 || iat_rva == 0 {
                return None;
            }
            let module = self.data_string(self.pe.image_base.checked_add(module_rva)?, MAX_MODULE_NAME)?.to_owned();
            let original_rva = if original == 0 { iat_rva } else { original };
            let original_va = self.pe.image_base.checked_add(original_rva)?;
            let iat_va = self.pe.image_base.checked_add(iat_rva)?;
            let mut terminated = false;
            for slot in 0..MAX_IMPORT_THUNKS {
                let delta = u32::try_from(slot.checked_mul(WORD_SIZE)?).ok()?;
                let thunk_va = original_va.checked_add(delta)?;
                let slot_va = iat_va.checked_add(delta)?;
                if !self.is_data(thunk_va, WORD_SIZE) || !self.is_data(slot_va, WORD_SIZE) {
                    return None;
                }
                let thunk = self.u32(thunk_va)?;
                let iat = self.u32(slot_va)?;
                if thunk == 0 {
                    if iat != 0 {
                        return None;
                    }
                    terminated = true;
                    break;
                }
                if iat == 0 {
                    return None;
                }
                let import = if thunk & ORDINAL_FLAG != 0 {
                    if thunk & !(ORDINAL_FLAG | ORDINAL_MASK) != 0 {
                        return None;
                    }
                    Import { module: module.clone(), name: None, ordinal: Some((thunk & ORDINAL_MASK) as u16) }
                } else {
                    let name_va = self.pe.image_base.checked_add(thunk)?;
                    if !self.is_data(name_va, IMPORT_HINT_SIZE as usize) {
                        return None;
                    }
                    let name = self.data_string(name_va.checked_add(IMPORT_HINT_SIZE)?, MAX_SYMBOL_NAME)?.to_owned();
                    Import { module: module.clone(), name: Some(name), ordinal: None }
                };
                found.push((slot_va, import));
            }
            if !terminated {
                return None;
            }
        }
        None
    }

    pub fn import(&self, va: u32) -> Option<Import> {
        let mut matches = self.imports()?.into_iter().filter(|(slot, _)| *slot == va);
        let (_, import) = matches.next()?;
        matches.next().is_none().then_some(import)
    }

    pub fn is_iat(&self, va: u32) -> bool {
        self.import(va).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::pe::tests::{synthetic_pe, TEST_IMAGE_BASE, TEST_SECTION_ALIGNMENT};

    const TEXT_SIZE: usize = 32;
    const RDATA_SIZE: usize = 1024;
    const DATA_SIZE: usize = 16;
    const TEXT_VA: u32 = TEST_IMAGE_BASE + TEST_SECTION_ALIGNMENT;
    const RDATA_VA: u32 = TEXT_VA + TEST_SECTION_ALIGNMENT;
    const DATA_VA: u32 = RDATA_VA + TEST_SECTION_ALIGNMENT;
    const FUNCTION_OFFSET: usize = 16;
    const RET: u8 = 0xc3;
    const TYPE_NAME: &str = ".?AVUOHuffmanCoding_S2C_Decompress@@";
    const COL: usize = 0x20;
    const TD: usize = 0x80;
    const CHD: usize = 0xc0;
    const BASE_ARRAY: usize = 0xe0;
    const BASE: usize = 0x100;
    const VTABLE: usize = 0x140;
    const TYPE_INFO_VTABLE: usize = 0x180;
    const DESCRIPTORS: usize = 0x200;
    const MODULE: usize = 0x240;
    const INT: usize = 0x260;
    const IAT: usize = 0x280;
    const IMPORT_NAME: usize = 0x2a0;
    const MODULE_NAME: &[u8] = b"WS2_32.dll\0";
    const SYMBOL_NAME: &[u8] = b"send\0";
    const HTONL_ORDINAL: u16 = 8;
    const BSS_SIZE: u32 = 32;
    const SECTION_UNINITIALIZED_DATA: u32 = 0x80;

    fn put(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + WORD_SIZE].copy_from_slice(&value.to_le_bytes());
    }

    fn fixture() -> (Vec<u8>, PeImage) {
        let mut text = vec![0x90; TEXT_SIZE];
        text[0] = CALL_REL32;
        put(&mut text, 1, (FUNCTION_OFFSET - REL32_SIZE) as u32);
        text[FUNCTION_OFFSET] = RET;
        let mut data = vec![0; RDATA_SIZE];
        put(&mut data, COL + COL_TYPE_OFFSET as usize, RDATA_VA + TD as u32);
        put(&mut data, COL + COL_HIERARCHY_OFFSET as usize, RDATA_VA + CHD as u32);
        put(&mut data, TD, RDATA_VA + TYPE_INFO_VTABLE as u32);
        let name = TD + TYPE_NAME_OFFSET as usize;
        data[name..name + TYPE_NAME.len()].copy_from_slice(TYPE_NAME.as_bytes());
        put(&mut data, CHD + HIERARCHY_COUNT_OFFSET as usize, 1);
        put(&mut data, CHD + HIERARCHY_BASES_OFFSET as usize, RDATA_VA + BASE_ARRAY as u32);
        put(&mut data, BASE_ARRAY, RDATA_VA + BASE as u32);
        put(&mut data, BASE, RDATA_VA + TD as u32);
        put(&mut data, VTABLE - WORD_SIZE, RDATA_VA + COL as u32);
        put(&mut data, VTABLE, TEXT_VA + FUNCTION_OFFSET as u32);
        put(&mut data, TYPE_INFO_VTABLE, TEXT_VA + FUNCTION_OFFSET as u32);
        put(&mut data, DESCRIPTORS, RDATA_VA - TEST_IMAGE_BASE + INT as u32);
        put(&mut data, DESCRIPTORS + IMPORT_NAME_OFFSET, RDATA_VA - TEST_IMAGE_BASE + MODULE as u32);
        put(&mut data, DESCRIPTORS + IMPORT_IAT_OFFSET, RDATA_VA - TEST_IMAGE_BASE + IAT as u32);
        data[MODULE..MODULE + MODULE_NAME.len()].copy_from_slice(MODULE_NAME);
        for table in [INT, IAT] {
            put(&mut data, table, ORDINAL_FLAG | u32::from(HTONL_ORDINAL));
            put(&mut data, table + WORD_SIZE, RDATA_VA - TEST_IMAGE_BASE + IMPORT_NAME as u32);
        }
        let name = IMPORT_NAME + IMPORT_HINT_SIZE as usize;
        data[name..name + SYMBOL_NAME.len()].copy_from_slice(SYMBOL_NAME);
        let mut bytes = synthetic_pe(&[(".text", text), (".rdata", data), (".data", vec![0; DATA_SIZE])], false);
        let nt = raw_u32(&bytes, DOS_PE_OFFSET).unwrap() as usize;
        let optional = nt + PE_SIGNATURE.len() + COFF_SIZE;
        let entry = optional + OPTIONAL_DIRECTORIES + IMPORT_DIRECTORY * DIRECTORY_SIZE;
        put(&mut bytes, entry, RDATA_VA - TEST_IMAGE_BASE + DESCRIPTORS as u32);
        put(&mut bytes, entry + WORD_SIZE, (IMPORT_DESCRIPTOR_SIZE * 2) as u32);
        let pe = PeImage::parse(&bytes).unwrap();
        (bytes, pe)
    }

    #[test]
    fn reads_and_rel32_edges_respect_sections_file_bounds_and_address_overflow() {
        let (bytes, pe) = fixture();
        let image = Image::new(&bytes, &pe);
        assert_eq!(image.call_target(TEXT_VA), Some(TEXT_VA + FUNCTION_OFFSET as u32));
        assert_eq!(image.jump_target(TEXT_VA), None);
        assert!(image.is_code(TEXT_VA));
        assert!(!image.is_code(RDATA_VA));
        assert!(image.is_data(RDATA_VA, RDATA_SIZE));
        assert!(!image.is_writable_data(RDATA_VA, WORD_SIZE));
        assert!(image.is_writable_data(DATA_VA, DATA_SIZE));
        assert!(image.read(TEXT_VA + TEXT_SIZE as u32 - 1, 2).is_none());
        assert!(image.read(u32::MAX, WORD_SIZE).is_none());
        assert!(Image::new(&bytes[..bytes.len() - 1], &pe).read(DATA_VA, DATA_SIZE).is_none());
        let mut changed = bytes.clone();
        let raw = pe.va_to_offset(TEXT_VA).unwrap();
        put(&mut changed, raw + 1, i32::MAX as u32);
        assert!(Image::new(&changed, &pe).call_target(TEXT_VA).is_none());
        let mut low = pe.clone();
        low.sections[0].virtual_address = 0;
        put(&mut changed, raw + 1, i32::MIN as u32);
        assert!(Image::new(&changed, &low).call_target(0).is_none());
        let mut high = pe.clone();
        high.sections[0].virtual_address = u32::MAX - TEXT_SIZE as u32;
        put(&mut changed, raw + 1, i32::MAX as u32);
        assert!(Image::new(&changed, &high).call_target(high.sections[0].virtual_address).is_none());
    }

    #[test]
    fn writable_globals_may_live_in_virtual_bss_but_cannot_cross_its_extent() {
        let (bytes, mut pe) = fixture();
        let section = pe.sections.iter_mut().find(|section| section.name == ".data").unwrap();
        section.virtual_size = section.raw_size + BSS_SIZE;
        section.characteristics &= !super::super::pe::SECTION_INITIALIZED_DATA;
        section.characteristics |= SECTION_UNINITIALIZED_DATA;
        let end = section.virtual_address + section.virtual_size;
        let bss = DATA_VA + DATA_SIZE as u32;
        let image = Image::new(&bytes, &pe);
        assert!(image.is_writable_data(bss, WORD_SIZE));
        assert!(image.is_writable_data(end - WORD_SIZE as u32, WORD_SIZE));
        assert!(!image.is_writable_data(end - WORD_SIZE as u32, WORD_SIZE + 1));
        assert!(!image.is_writable_data(end, WORD_SIZE));
        assert!(!image.is_writable_data(u32::MAX, WORD_SIZE));
        assert!(image.read(bss, WORD_SIZE).is_none());
        assert!(!image.is_data(bss, WORD_SIZE));
    }

    #[test]
    fn rtti_checks_locator_type_hierarchy_and_vtable_targets() {
        let (bytes, pe) = fixture();
        let vtable = RDATA_VA + VTABLE as u32;
        let image = Image::new(&bytes, &pe);
        assert_eq!(image.rtti_type(vtable), Some(TYPE_NAME));
        assert_eq!(image.vtables(TYPE_NAME), [vtable]);
        assert!(image.vtables(".?AVOther@@").is_empty());
        let raw = pe.va_to_offset(RDATA_VA).unwrap();
        for (at, value) in [
            (VTABLE, RDATA_VA),
            (VTABLE - WORD_SIZE, u32::MAX),
            (COL, 1),
            (COL + COL_TYPE_OFFSET as usize, TEXT_VA),
            (TD, u32::MAX),
            (CHD + HIERARCHY_COUNT_OFFSET as usize, MAX_BASE_CLASSES + 1),
            (CHD + HIERARCHY_BASES_OFFSET as usize, u32::MAX),
            (BASE, RDATA_VA),
        ] {
            let mut invalid = bytes.clone();
            put(&mut invalid, raw + at, value);
            assert!(Image::new(&invalid, &pe).rtti_type(vtable).is_none(), "field {at:#x}");
        }
    }

    #[test]
    fn code_scan_matches_wildcards_without_scanning_data() {
        let (bytes, pe) = fixture();
        let image = Image::new(&bytes, &pe);
        assert_eq!(image.scan(&[u16::from(CALL_REL32), PATTERN_WILDCARD, 0, 0, 0]), [TEXT_VA]);
        assert!(image.scan(&[]).is_empty());
        assert!(image.scan(&[PATTERN_WILDCARD + 1]).is_empty());
        assert!(image.scan(&TYPE_NAME.as_bytes().iter().map(|byte| u16::from(*byte)).collect::<Vec<_>>()).is_empty());
    }

    #[test]
    fn resolves_named_and_ordinal_imports_and_original_thunk_fallback() {
        let (bytes, pe) = fixture();
        let image = Image::new(&bytes, &pe);
        assert_eq!(image.import(RDATA_VA + IAT as u32), Some(Import {
            module: "WS2_32.dll".to_owned(), name: None, ordinal: Some(HTONL_ORDINAL),
        }));
        assert_eq!(image.import(RDATA_VA + IAT as u32 + WORD_SIZE as u32), Some(Import {
            module: "WS2_32.dll".to_owned(), name: Some("send".to_owned()), ordinal: None,
        }));
        assert!(image.is_iat(RDATA_VA + IAT as u32));
        assert!(!image.is_iat(RDATA_VA + IAT as u32 + 1));
        assert!(!image.is_iat(TEXT_VA));
        let mut fallback = bytes;
        let raw = pe.va_to_offset(RDATA_VA + DESCRIPTORS as u32).unwrap();
        put(&mut fallback, raw, 0);
        assert!(Image::new(&fallback, &pe).is_iat(RDATA_VA + IAT as u32));
    }

    #[test]
    fn corrupt_import_descriptors_thunks_names_and_terminators_are_refused() {
        let (bytes, pe) = fixture();
        let raw = pe.va_to_offset(RDATA_VA).unwrap();
        for (at, value) in [
            (DESCRIPTORS, u32::MAX),
            (DESCRIPTORS + IMPORT_NAME_OFFSET, u32::MAX),
            (DESCRIPTORS + IMPORT_IAT_OFFSET, u32::MAX),
            (DESCRIPTORS + IMPORT_DESCRIPTOR_SIZE, 1),
            (INT, ORDINAL_FLAG | (ORDINAL_MASK + 1)),
            (INT, u32::MAX & !ORDINAL_FLAG),
            (IAT, 0),
            (IAT + WORD_SIZE * 2, 1),
        ] {
            let mut invalid = bytes.clone();
            put(&mut invalid, raw + at, value);
            assert!(!Image::new(&invalid, &pe).is_iat(RDATA_VA + IAT as u32), "field {at:#x}");
        }
        let mut invalid = bytes;
        invalid[raw + MODULE..raw + RDATA_SIZE].fill(b'x');
        assert!(!Image::new(&invalid, &pe).is_iat(RDATA_VA + IAT as u32));
    }
}
