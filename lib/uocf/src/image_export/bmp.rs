//! The 16-bit BMP of a Classic image: a `BITMAPINFOHEADER` with `BI_RGB` compression, which at 16 bits per pixel
//! is X1R5G5B5, the Classic client's own colour word. The words are written bit for bit (bit 15 too, which readers
//! ignore), rows bottom-up and padded to 4 bytes; [`decode_rgb555`] reads such a file back to the same words.
//! The 8-bit and 32-bit BMPs come from the `image` crate's encoder (see the module above).

crate::eyre_imports!();

const SIGNATURE: [u8; 2] = *b"BM";
const FILE_HEADER_BYTES: usize = 14;
const INFO_HEADER_BYTES: usize = 40;
const PLANES: u16 = 1;
const RGB555_BITS_PER_PIXEL: u16 = 16;
/// No compression; at 16 bits per pixel the words are X1R5G5B5.
const BI_RGB: u32 = 0;
/// Pixels per metre left unset, as the `image` crate's encoder does.
const UNSET_RESOLUTION: i32 = 0;
/// A palette size of 0: none (at 16 bits) or the full table.
const NO_PALETTE_COLOURS: u32 = 0;
const ROW_ALIGNMENT_BYTES: usize = 4;
const WORD_BYTES: usize = size_of::<u16>();
/// Offsets of the fields read back.
const FILE_PIXELS_OFFSET_AT: usize = 10;
const INFO_SIZE_AT: usize = FILE_HEADER_BYTES;
const WIDTH_AT: usize = FILE_HEADER_BYTES + 4;
const HEIGHT_AT: usize = FILE_HEADER_BYTES + 8;
const BITS_PER_PIXEL_AT: usize = FILE_HEADER_BYTES + 14;
const COMPRESSION_AT: usize = FILE_HEADER_BYTES + 16;

fn padded_row_bytes(width: usize) -> usize {
    (width * WORD_BYTES).next_multiple_of(ROW_ALIGNMENT_BYTES)
}

/// A 16-bit BMP of `width x height` X1R5G5B5 `words`, row by row from the top.
pub fn encode_rgb555(width: u32, height: u32, words: &[u16]) -> eyre::Result<Vec<u8>> {
    let (width_px, height_px) = (width as usize, height as usize);
    eyre::ensure!(width > 0 && height > 0, "a {width}x{height} image has no pixels");
    eyre::ensure!(words.len() == width_px * height_px, "{} colours for {width}x{height}", words.len());
    let row_bytes = padded_row_bytes(width_px);
    let pixels_offset = FILE_HEADER_BYTES + INFO_HEADER_BYTES;
    let image_bytes = row_bytes * height_px;
    let file_bytes = u32::try_from(pixels_offset + image_bytes).map_err(|_| eyre!("a {width}x{height} BMP passes 4 GiB"))?;
    let (Ok(width_field), Ok(height_field)) = (i32::try_from(width), i32::try_from(height)) else {
        eyre::bail!("a {width}x{height} image is larger than a BMP holds");
    };
    let mut out = Vec::with_capacity(file_bytes as usize);
    out.extend_from_slice(&SIGNATURE);
    out.extend_from_slice(&file_bytes.to_le_bytes());
    out.extend_from_slice(&[0; 2 * WORD_BYTES]);
    out.extend_from_slice(&(pixels_offset as u32).to_le_bytes());
    out.extend_from_slice(&(INFO_HEADER_BYTES as u32).to_le_bytes());
    out.extend_from_slice(&width_field.to_le_bytes());
    out.extend_from_slice(&height_field.to_le_bytes());
    out.extend_from_slice(&PLANES.to_le_bytes());
    out.extend_from_slice(&RGB555_BITS_PER_PIXEL.to_le_bytes());
    out.extend_from_slice(&BI_RGB.to_le_bytes());
    out.extend_from_slice(&(image_bytes as u32).to_le_bytes());
    out.extend_from_slice(&UNSET_RESOLUTION.to_le_bytes());
    out.extend_from_slice(&UNSET_RESOLUTION.to_le_bytes());
    out.extend_from_slice(&NO_PALETTE_COLOURS.to_le_bytes());
    out.extend_from_slice(&NO_PALETTE_COLOURS.to_le_bytes());
    for row in words.chunks_exact(width_px).rev() {
        let start = out.len();
        out.extend(row.iter().flat_map(|word| word.to_le_bytes()));
        out.resize(start + row_bytes, 0);
    }
    Ok(out)
}

/// The `(width, height, words)` of a 16-bit `BI_RGB` BMP, rows from the top; `None` for any other file.
pub fn decode_rgb555(bytes: &[u8]) -> eyre::Result<Option<(u32, u32, Vec<u16>)>> {
    let u16_at = |at: usize| bytes.get(at..at + WORD_BYTES).map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    let u32_at = |at: usize| bytes.get(at..at + size_of::<u32>()).map(|four| u32::from_le_bytes(four.try_into().expect("4 bytes")));
    if bytes.get(..SIGNATURE.len()) != Some(&SIGNATURE[..])
        || u16_at(BITS_PER_PIXEL_AT) != Some(RGB555_BITS_PER_PIXEL)
        || u32_at(COMPRESSION_AT) != Some(BI_RGB)
        || u32_at(INFO_SIZE_AT).is_none_or(|size| (size as usize) < INFO_HEADER_BYTES)
    {
        return Ok(None);
    }
    let (Some(offset), Some(width), Some(height)) = (u32_at(FILE_PIXELS_OFFSET_AT), u32_at(WIDTH_AT), u32_at(HEIGHT_AT)) else {
        return Ok(None);
    };
    let (width, height) = (width as i32, height as i32);
    eyre::ensure!(width > 0 && height != 0, "a 16-bit BMP of {width}x{height}");
    // A negative height stores the rows top-down.
    let (width_px, height_px, top_down) = (width as usize, height.unsigned_abs() as usize, height < 0);
    let row_bytes = padded_row_bytes(width_px);
    let pixels = bytes
        .get(offset as usize..)
        .filter(|pixels| pixels.len() >= row_bytes * height_px)
        .ok_or_else(|| eyre!("a {width_px}x{height_px} 16-bit BMP of {} bytes ends before its pixels", bytes.len()))?;
    let mut words = Vec::with_capacity(width_px * height_px);
    for y in 0..height_px {
        let stored_row = if top_down { y } else { height_px - 1 - y };
        let row = &pixels[stored_row * row_bytes..][..width_px * WORD_BYTES];
        words.extend(row.chunks_exact(WORD_BYTES).map(|pair| u16::from_le_bytes([pair[0], pair[1]])));
    }
    Ok(Some((width_px as u32, height_px as u32, words)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_16_bit_bmp_reads_back_bit_for_bit() {
        // An odd width pads each row.
        let words = [0x0000, 0x7FFF, 0x8001, 0x7C00, 0x03E0, 0x001F];
        let bmp = encode_rgb555(3, 2, &words).unwrap();
        assert_eq!(bmp.len(), FILE_HEADER_BYTES + INFO_HEADER_BYTES + 2 * padded_row_bytes(3));
        assert_eq!(decode_rgb555(&bmp).unwrap(), Some((3, 2, words.to_vec())));
        // The `image` crate reads it as X1R5G5B5 too, with its own 5 to 8 bit widening.
        let decoded = image::load_from_memory(&bmp).unwrap().to_rgb8();
        assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255]);
        assert_eq!(decoded.get_pixel(0, 1).0, [255, 0, 0]);
    }

    #[test]
    fn other_files_are_not_16_bit_bmps() {
        assert_eq!(decode_rgb555(b"\x89PNG\r\n").unwrap(), None);
        let mut rgba = Vec::new();
        image::codecs::bmp::BmpEncoder::new(&mut rgba).encode(&[1, 2, 3, 4], 1, 1, image::ExtendedColorType::Rgba8).unwrap();
        assert_eq!(decode_rgb555(&rgba).unwrap(), None);
    }
}
