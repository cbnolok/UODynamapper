//! Keep PE32 HIGHLOW relocations consistent with generated KR code, without moving unrelated table blocks.

crate::eyre_imports!();

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::pe::{PeImage, read_u32};

const WORD_SIZE: usize = size_of::<u32>();
const ENTRY_SIZE: usize = size_of::<u16>();
const BLOCK_HEADER_SIZE: usize = WORD_SIZE * 2;
const BLOCK_ALIGNMENT: usize = WORD_SIZE;
const PAGE_SIZE: u32 = 0x1000;
const OFFSET_MASK: u16 = 0x0fff;
const TYPE_SHIFT: u32 = 12;
const TYPE_ABSOLUTE: u16 = 0;
const TYPE_HIGHLOW: u16 = 3;
const SECTION_DISCARDABLE: u32 = 0x0200_0000;
const DIRECTORY_COUNT_FROM_CHECKSUM: usize = 28;
const DIRECTORIES_FROM_CHECKSUM: usize = 32;
const DIRECTORY_ENTRY_SIZE: usize = WORD_SIZE * 2;
const STANDARD_DIRECTORY_COUNT: usize = 16;
const SECURITY_DIRECTORY: usize = 4;
const RELOCATION_DIRECTORY: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelocationWrite {
    pub offset: usize,
    pub new: Vec<u8>,
}

struct Entry {
    offset: usize,
    page: u32,
    word: u16,
    target: Option<usize>,
}

fn mapped_word(pe: &PeImage, bytes: &[u8], rva: u32) -> eyre::Result<usize> {
    let va = pe.image_base.checked_add(rva).ok_or_else(|| eyre!("base relocation address overflows"))?;
    let end = va.checked_add((WORD_SIZE - 1) as u32).ok_or_else(|| eyre!("base relocation range overflows"))?;
    let section = pe.section_of_va(va).ok_or_else(|| eyre!("base relocation target is outside a section"))?;
    eyre::ensure!(end.checked_sub(section.virtual_address).is_some_and(|delta| delta < section.raw_size),
        "base relocation target {va:#x} crosses a section");
    let at = pe.va_to_offset(va).ok_or_else(|| eyre!("base relocation target {va:#x} is not file-backed"))?;
    eyre::ensure!(pe.va_to_offset(end) == at.checked_add(WORD_SIZE - 1) && bytes.get(at..at + WORD_SIZE).is_some(),
        "base relocation target {va:#x} crosses a file-backed section");
    Ok(at)
}

/// Replace relocations in `span` with exactly the absolute DWORD operands of the generated code.
/// Existing entries outside the span keep their positions and values. Empty slots are reused before appending
/// a block into verified zero padding. This also repairs earlier copies whose code is already patched.
pub(crate) fn plan(
    bytes: &[u8], pe: &PeImage, span: Range<usize>, absolute_operands: &[usize],
) -> eyre::Result<Vec<RelocationWrite>> {
    let (rva, size) = pe.base_relocation_directory;
    if rva == 0 && size == 0 {
        return Ok(Vec::new());
    }
    eyre::ensure!(rva != 0 && size != 0, "incomplete PE base relocation directory");
    eyre::ensure!(span.start <= span.end && span.end <= bytes.len(), "invalid generated-code range for base relocations");
    let mut desired = BTreeSet::new();
    for &at in absolute_operands {
        eyre::ensure!(at >= span.start && at.checked_add(WORD_SIZE).is_some_and(|end| end <= span.end),
            "generated absolute operand {at:#x} is outside the handshake replacement");
        let va = pe.offset_to_va(at).ok_or_else(|| eyre!("generated absolute operand is outside a PE section"))?;
        let operand_rva = va.checked_sub(pe.image_base).ok_or_else(|| eyre!("generated operand precedes the image"))?;
        eyre::ensure!(mapped_word(pe, bytes, operand_rva)? == at && desired.insert(at), "duplicate generated absolute operand");
    }
    let va = pe.image_base.checked_add(rva).ok_or_else(|| eyre!("base relocation directory address overflows"))?;
    let section = pe.section_of_va(va).ok_or_else(|| eyre!("base relocation directory is not file-backed"))?;
    let start = pe.va_to_offset(va).ok_or_else(|| eyre!("base relocation directory is not mapped"))?;
    let end = start.checked_add(size as usize).ok_or_else(|| eyre!("base relocation directory size overflows"))?;
    eyre::ensure!(end <= section.raw_range().end && end <= bytes.len(), "base relocation directory crosses its section");
    let mut cursor = start;
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    while cursor < end {
        if bytes[cursor..end].iter().all(|byte| *byte == 0) {
            break;
        }
        eyre::ensure!(end - cursor >= BLOCK_HEADER_SIZE, "truncated PE base relocation block");
        let page = read_u32(bytes, cursor)?;
        let block_size = read_u32(bytes, cursor + WORD_SIZE)? as usize;
        eyre::ensure!(page % PAGE_SIZE == 0 && block_size >= BLOCK_HEADER_SIZE && block_size % BLOCK_ALIGNMENT == 0,
            "invalid PE base relocation block at {cursor:#x}");
        let next = cursor.checked_add(block_size).ok_or_else(|| eyre!("base relocation block overflows"))?;
        eyre::ensure!(next <= end, "PE base relocation block exceeds its directory");
        for at in (cursor + BLOCK_HEADER_SIZE..next).step_by(ENTRY_SIZE) {
            let word = u16::from_le_bytes(bytes[at..at + ENTRY_SIZE].try_into().expect("relocation entry"));
            let kind = word >> TYPE_SHIFT;
            let target = match kind {
                TYPE_ABSOLUTE => None,
                TYPE_HIGHLOW => {
                    let target_rva = page.checked_add(u32::from(word & OFFSET_MASK)).ok_or_else(|| eyre!("base relocation RVA overflows"))?;
                    eyre::ensure!(seen.insert(target_rva), "duplicate PE base relocation target {target_rva:#x}");
                    Some(mapped_word(pe, bytes, target_rva)?)
                }
                _ => eyre::bail!("unsupported PE32 base relocation type {kind}; the handshake is not patched"),
            };
            entries.push(Entry { offset: at, page, word, target });
        }
        cursor = next;
    }
    let mut writes = Vec::new();
    let existing_targets: Vec<_> = entries.iter().filter_map(|entry| entry.target).collect();
    let mut present = BTreeSet::new();
    for entry in &mut entries {
        let Some(at) = entry.target else { continue };
        let overlaps = at < span.end && at + WORD_SIZE > span.start;
        if !overlaps { continue; }
        eyre::ensure!(at >= span.start && at + WORD_SIZE <= span.end, "base relocation partly overlaps the handshake boundary");
        if desired.contains(&at) {
            present.insert(at);
        } else {
            entry.word = 0;
            entry.target = None;
        }
    }
    let mut appended = BTreeMap::<u32, Vec<u16>>::new();
    for at in desired.difference(&present) {
        let va = pe.offset_to_va(*at).expect("checked generated operand");
        let target = va - pe.image_base;
        let page = target / PAGE_SIZE * PAGE_SIZE;
        let word = (TYPE_HIGHLOW << TYPE_SHIFT) | (target % PAGE_SIZE) as u16;
        if let Some(slot) = entries.iter_mut().find(|entry| entry.page == page && entry.target.is_none()) {
            slot.word = word;
            slot.target = Some(*at);
        } else {
            appended.entry(page).or_default().push(word);
        }
    }
    for entry in entries {
        if bytes[entry.offset..entry.offset + ENTRY_SIZE] != entry.word.to_le_bytes() {
            writes.push(RelocationWrite { offset: entry.offset, new: entry.word.to_le_bytes().to_vec() });
        }
    }
    let append_start = cursor;
    let mut append = Vec::new();
    for (page, words) in appended {
        let length = (BLOCK_HEADER_SIZE + words.len() * ENTRY_SIZE).next_multiple_of(BLOCK_ALIGNMENT);
        append.extend(page.to_le_bytes());
        append.extend(u32::try_from(length)?.to_le_bytes());
        for word in words { append.extend(word.to_le_bytes()); }
        append.resize(append.len().next_multiple_of(BLOCK_ALIGNMENT), 0);
    }
    if !append.is_empty() {
        let append_end = append_start.checked_add(append.len()).ok_or_else(|| eyre!("base relocation append overflows"))?;
        eyre::ensure!(append_end <= section.raw_range().end && append_end <= bytes.len()
                && bytes[append_start..append_end].iter().all(|byte| *byte == 0),
            "no zero padding is available for the generated code's PE base relocations");
        eyre::ensure!(append_end <= end || (section.name == ".reloc" && section.characteristics & SECTION_DISCARDABLE != 0),
            "base relocations can only grow into a dedicated discardable .reloc section");
        eyre::ensure!(!existing_targets.iter().any(|at| *at < append_end && *at + WORD_SIZE > append_start),
            "base relocation growth would overwrite another relocation's target");
        let count = read_u32(bytes, pe.checksum_offset + DIRECTORY_COUNT_FROM_CHECKSUM)? as usize;
        eyre::ensure!(count <= STANDARD_DIRECTORY_COUNT, "cannot verify padding with nonstandard PE data directories");
        for index in 0..count {
            if index == RELOCATION_DIRECTORY { continue; }
            let entry = pe.checksum_offset + DIRECTORIES_FROM_CHECKSUM + index * DIRECTORY_ENTRY_SIZE;
            let address = read_u32(bytes, entry)?;
            let length = read_u32(bytes, entry + WORD_SIZE)? as usize;
            if address == 0 || length == 0 { continue; }
            let offset = if index == SECURITY_DIRECTORY { Some(address as usize) } else {
                pe.image_base.checked_add(address).and_then(|va| pe.va_to_offset(va))
            }.ok_or_else(|| eyre!("cannot verify PE directory {index} while growing base relocations"))?;
            let protected_end = offset.checked_add(length).ok_or_else(|| eyre!("PE directory range overflows"))?;
            eyre::ensure!(protected_end <= bytes.len() && !(offset < append_end && protected_end > append_start),
                "base relocation growth would overlap another PE data directory");
        }
        writes.push(RelocationWrite { offset: append_start, new: append });
        let new_size = u32::try_from(append_end - start)?.max(size);
        if new_size != size {
            writes.push(RelocationWrite {
                offset: pe.base_relocation_directory_entry_offset() + WORD_SIZE,
                new: new_size.to_le_bytes().to_vec(),
            });
        }
    }
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::pe::tests::synthetic_pe;

    const TEXT_SIZE: usize = 0x3000;
    const RELOC_CAPACITY: usize = 0x100;
    const TARGET: usize = 0x100;
    const OTHER_TARGET: usize = 0x300;
    const SPAN_LENGTH: usize = 0x20;

    fn fixture(words: &[u16], capacity: usize) -> (Vec<u8>, PeImage, usize, usize) {
        let mut bytes = synthetic_pe(&[(".text", vec![0x90; TEXT_SIZE]), (".reloc", vec![0; capacity])], false);
        let pe = PeImage::parse(&bytes).unwrap();
        const SECTION_FLAGS_OFFSET: usize = 36;
        let header = bytes.windows(8).position(|window| window == b".reloc\0\0").unwrap();
        let flags = read_u32(&bytes, header + SECTION_FLAGS_OFFSET).unwrap() | SECTION_DISCARDABLE;
        bytes[header + SECTION_FLAGS_OFFSET..][..WORD_SIZE].copy_from_slice(&flags.to_le_bytes());
        let text = pe.section(".text").unwrap();
        let page = text.virtual_address - pe.image_base;
        let text_offset = text.raw_offset as usize;
        let table = pe.section(".reloc").unwrap();
        let table_rva = table.virtual_address - pe.image_base;
        let table_offset = table.raw_offset as usize;
        let length = (BLOCK_HEADER_SIZE + words.len() * ENTRY_SIZE).next_multiple_of(BLOCK_ALIGNMENT);
        assert!(length <= capacity);
        bytes[table_offset..][..WORD_SIZE].copy_from_slice(&page.to_le_bytes());
        bytes[table_offset + WORD_SIZE..][..WORD_SIZE].copy_from_slice(&(length as u32).to_le_bytes());
        for (index, word) in words.iter().enumerate() {
            let at = table_offset + BLOCK_HEADER_SIZE + index * ENTRY_SIZE;
            bytes[at..][..ENTRY_SIZE].copy_from_slice(&word.to_le_bytes());
        }
        let header = pe.base_relocation_directory_entry_offset();
        bytes[header..][..WORD_SIZE].copy_from_slice(&table_rva.to_le_bytes());
        bytes[header + WORD_SIZE..][..WORD_SIZE].copy_from_slice(&(length as u32).to_le_bytes());
        let pe = PeImage::parse(&bytes).unwrap();
        (bytes, pe, text_offset, table_offset)
    }

    fn apply(bytes: &[u8], writes: &[RelocationWrite]) -> Vec<u8> {
        let mut result = bytes.to_vec();
        for write in writes { result[write.offset..][..write.new.len()].copy_from_slice(&write.new); }
        result
    }

    #[test]
    fn stale_operands_are_replaced_in_place_and_other_relocations_are_preserved() {
        let words = [0x3100, 0x3110, 0x3300, 0];
        let (bytes, pe, code, table) = fixture(&words, RELOC_CAPACITY);
        let span = code + TARGET..code + TARGET + SPAN_LENGTH;
        let desired = [code + TARGET + WORD_SIZE * 2];
        let writes = plan(&bytes, &pe, span.clone(), &desired).unwrap();
        assert_eq!(writes.len(), 2);
        let after = apply(&bytes, &writes);
        assert_eq!(&after[table + BLOCK_HEADER_SIZE..][..ENTRY_SIZE], &0x3108u16.to_le_bytes());
        assert_eq!(&after[table + BLOCK_HEADER_SIZE + ENTRY_SIZE..][..ENTRY_SIZE], &0u16.to_le_bytes());
        assert_eq!(&after[table + BLOCK_HEADER_SIZE + ENTRY_SIZE * 2..][..ENTRY_SIZE], &words[2].to_le_bytes());
        assert_eq!(after[code + OTHER_TARGET..code + OTHER_TARGET + WORD_SIZE], bytes[code + OTHER_TARGET..code + OTHER_TARGET + WORD_SIZE]);
        assert!(plan(&after, &PeImage::parse(&after).unwrap(), span, &desired).unwrap().is_empty());
    }

    #[test]
    fn a_missing_page_block_uses_zero_padding_and_updates_the_directory_size() {
        let (bytes, pe, code, table) = fixture(&[0x3100, 0], RELOC_CAPACITY);
        let span = code + PAGE_SIZE as usize + TARGET..code + PAGE_SIZE as usize + TARGET + SPAN_LENGTH;
        let desired = [span.start + WORD_SIZE * 2];
        let writes = plan(&bytes, &pe, span.clone(), &desired).unwrap();
        assert_eq!(writes.len(), 2);
        let after = apply(&bytes, &writes);
        let after_pe = PeImage::parse(&after).unwrap();
        assert_eq!(after_pe.base_relocation_directory.1, 24);
        assert_eq!(read_u32(&after, table + 12).unwrap(), PAGE_SIZE * 2);
        assert_eq!(&after[table + 12 + BLOCK_HEADER_SIZE..][..ENTRY_SIZE], &0x3108u16.to_le_bytes());
        assert!(plan(&after, &after_pe, span, &desired).unwrap().is_empty());
    }

    #[test]
    fn no_directory_needs_no_bookkeeping_and_insufficient_capacity_refuses_the_patch() {
        let bytes = synthetic_pe(&[(".text", vec![0; TEXT_SIZE])], false);
        let pe = PeImage::parse(&bytes).unwrap();
        assert!(plan(&bytes, &pe, 0..0, &[]).unwrap().is_empty());
        let (bytes, pe, code, _) = fixture(&[0x3100, 0], 12);
        let start = code + PAGE_SIZE as usize + TARGET;
        assert!(plan(&bytes, &pe, start..start + SPAN_LENGTH, &[start + WORD_SIZE * 2]).unwrap_err().to_string().contains("zero padding"));
    }

    #[test]
    fn malformed_tables_and_partial_operand_overlaps_are_rejected() {
        for words in [[0x4100, 0], [0x3100, 0x3100]] {
            let (bytes, pe, code, _) = fixture(&words, RELOC_CAPACITY);
            assert!(plan(&bytes, &pe, code + TARGET..code + TARGET + SPAN_LENGTH, &[]).is_err());
        }
        let (mut bytes, pe, code, table) = fixture(&[0x3100, 0], RELOC_CAPACITY);
        assert!(plan(&bytes, &pe, code + TARGET + 1..code + TARGET + SPAN_LENGTH, &[]).is_err());
        assert!(plan(&bytes, &pe, code + TARGET..code + TARGET + SPAN_LENGTH, &[code + TARGET - 1]).is_err());
        bytes[table + WORD_SIZE..][..WORD_SIZE].copy_from_slice(&10u32.to_le_bytes());
        assert!(plan(&bytes, &pe, code + TARGET..code + TARGET + SPAN_LENGTH, &[]).is_err());
    }

    #[test]
    fn append_does_not_overwrite_zero_filled_relocation_targets_or_other_directories() {
        // The zero-filled append area is itself the target of an unrelated HIGHLOW relocation.
        let (mut bytes, pe, code, table) = fixture(&[0x300c, 0], RELOC_CAPACITY);
        let table_page = pe.section(".reloc").unwrap().virtual_address - pe.image_base;
        bytes[table..][..WORD_SIZE].copy_from_slice(&table_page.to_le_bytes());
        let start = code + TARGET;
        assert!(plan(&bytes, &pe, start..start + SPAN_LENGTH, &[start + WORD_SIZE * 2])
            .unwrap_err().to_string().contains("another relocation's target"));

        let (mut bytes, pe, code, table) = fixture(&[0x3100, 0], RELOC_CAPACITY);
        let append_start = table + 12;
        const DEBUG_DIRECTORY: usize = 6;
        let directory = pe.checksum_offset + DIRECTORIES_FROM_CHECKSUM + DEBUG_DIRECTORY * DIRECTORY_ENTRY_SIZE;
        let rva = pe.offset_to_va(append_start).unwrap() - pe.image_base;
        bytes[directory..][..WORD_SIZE].copy_from_slice(&rva.to_le_bytes());
        bytes[directory + WORD_SIZE..][..WORD_SIZE].copy_from_slice(&(WORD_SIZE as u32).to_le_bytes());
        let start = code + PAGE_SIZE as usize + TARGET;
        assert!(plan(&bytes, &pe, start..start + SPAN_LENGTH, &[start + WORD_SIZE * 2])
            .unwrap_err().to_string().contains("another PE data directory"));
    }
}
