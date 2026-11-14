//! Pure-std decoration of resolver fixtures for library and GUI patcher tests.
//! Preserve the validated helper graph and imports, adding cipher loops in reserved text and a facet section.

pub const CONNECT_OFFSET: usize = 0x700;
pub const CIPHER_TEXT_OFFSET: usize = 0x200;
const PE_POINTER: usize = 0x3c;
const PE_SIGNATURE_SIZE: usize = 4;
const COFF_SIZE: usize = 20;
const COFF_SECTION_COUNT: usize = 2;
const COFF_OPTIONAL_SIZE: usize = 16;
const OPTIONAL_IMAGE_BASE: usize = 28;
const OPTIONAL_CHECKSUM: usize = 64;
const OPTIONAL_IMAGE_SIZE: usize = 56;
const SECTION_SIZE: usize = 40;
const SECTION_VIRTUAL_SIZE: usize = 8;
const SECTION_RVA: usize = 12;
const SECTION_RAW_SIZE: usize = 16;
const SECTION_RAW: usize = 20;
const SECTION_FLAGS: usize = 36;
const SECTION_ALIGNMENT: usize = 0x1000;
const FACET_FLAGS: u32 = 0xc000_0040;
const FACET_RECORD_SIZE: usize = 12;
const FACET_PADDING: usize = 16;
const DWORD_SIZE: usize = size_of::<u32>();
const CHECKSUM_MASK: u64 = 0xffff;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + size_of::<u16>()].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + DWORD_SIZE].try_into().unwrap())
}

fn put(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + DWORD_SIZE].copy_from_slice(&value.to_le_bytes());
}

pub fn client_image(mut bytes: Vec<u8>, ciphers: &[u8], facets: &[(&str, u32, u32)]) -> Vec<u8> {
    let coff = u32_at(&bytes, PE_POINTER) as usize + PE_SIGNATURE_SIZE;
    let optional = coff + COFF_SIZE;
    let image_base = u32_at(&bytes, optional + OPTIONAL_IMAGE_BASE);
    let count = usize::from(u16_at(&bytes, coff + COFF_SECTION_COUNT));
    let sections = optional + usize::from(u16_at(&bytes, coff + COFF_OPTIONAL_SIZE));
    let mut end_rva = 0;
    let mut first_raw = usize::MAX;
    let mut text = None;
    for number in 0..count {
        let at = sections + number * SECTION_SIZE;
        let raw = u32_at(&bytes, at + SECTION_RAW) as usize;
        first_raw = first_raw.min(raw);
        let extent = u32_at(&bytes, at + SECTION_VIRTUAL_SIZE).max(u32_at(&bytes, at + SECTION_RAW_SIZE));
        end_rva = end_rva.max(u32_at(&bytes, at + SECTION_RVA) + extent);
        if &bytes[at..at + 5] == b".text" {
            text = Some((raw, u32_at(&bytes, at + SECTION_RAW_SIZE) as usize));
        }
    }
    let (text_raw, text_size) = text.unwrap();
    assert!(CIPHER_TEXT_OFFSET + ciphers.len() <= text_size);
    let destination = text_raw + CIPHER_TEXT_OFFSET;
    assert!(bytes[destination..destination + ciphers.len()].iter().all(|byte| matches!(byte, 0x90 | 0xcc)),
        "cipher fixtures must not overwrite validated helper code");
    bytes[destination..destination + ciphers.len()].copy_from_slice(ciphers);

    let rva = (end_rva as usize).next_multiple_of(SECTION_ALIGNMENT) as u32;
    let raw = bytes.len().next_multiple_of(SECTION_ALIGNMENT);
    let mut content = vec![0; FACET_PADDING];
    let mut pointers = Vec::new();
    for (name, _, _) in facets {
        pointers.push(image_base + rva + content.len() as u32);
        content.extend_from_slice(name.as_bytes());
        content.push(0);
    }
    content.resize(content.len().next_multiple_of(DWORD_SIZE) + FACET_PADDING, 0);
    for ((_, width, height), pointer) in facets.iter().zip(pointers) {
        for value in [pointer, *width, *height] {
            content.extend_from_slice(&value.to_le_bytes());
        }
    }
    content.extend([0; FACET_RECORD_SIZE + FACET_PADDING]);
    let header = sections + count * SECTION_SIZE;
    assert!(header + SECTION_SIZE <= first_raw);
    bytes[header..header + SECTION_SIZE].fill(0);
    bytes[header..header + 7].copy_from_slice(b".facets");
    for (field, value) in [
        (SECTION_VIRTUAL_SIZE, content.len() as u32), (SECTION_RVA, rva),
        (SECTION_RAW_SIZE, content.len() as u32), (SECTION_RAW, raw as u32), (SECTION_FLAGS, FACET_FLAGS),
    ] {
        put(&mut bytes, header + field, value);
    }
    bytes[coff + COFF_SECTION_COUNT..coff + COFF_SECTION_COUNT + size_of::<u16>()]
        .copy_from_slice(&(count as u16 + 1).to_le_bytes());
    put(&mut bytes, optional + OPTIONAL_IMAGE_SIZE,
        (rva as usize + content.len()).next_multiple_of(SECTION_ALIGNMENT) as u32);
    bytes.resize(raw, 0);
    bytes.extend(content);
    let checksum = optional + OPTIONAL_CHECKSUM;
    put(&mut bytes, checksum, 0);
    let mut sum = 0u64;
    for chunk in bytes.chunks(size_of::<u16>()) {
        let word = u16::from_le_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]);
        sum += u64::from(word);
        sum = (sum & CHECKSUM_MASK) + (sum >> u16::BITS);
    }
    sum = (sum & CHECKSUM_MASK) + (sum >> u16::BITS);
    let value = (sum & CHECKSUM_MASK) as u32 + bytes.len() as u32;
    put(&mut bytes, checksum, value);
    bytes
}
