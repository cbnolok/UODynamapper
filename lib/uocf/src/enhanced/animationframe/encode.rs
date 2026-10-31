//! AMOU writer: the container ([`AmouFile`]), the frame RLE encoder ([`encode_frame_rle`]) and the indexed frame
//! form both work on ([`IndexedFrame`]).
//!
//! The rules come from the client decoders (`UOAnimations::DecodeFrameRle` / `FindFrameEntry` in UOSA.exe and
//! UOKR.exe, `scripts/dev-helpers/ec-research/ghidra-animation-frames/`) and from the shipped files, which this
//! writer reproduces byte for byte when given their palette (see `ENHANCED_CLIENT_FILES.md`, AnimationFrame).

use color_eyre::eyre::{self, WrapErr};

use super::{
    AnimationBounds, AnimationFrame, FrameEntry, EDGE_FACTOR_BITS, EDGE_FACTOR_MASK, EDGE_FACTOR_ONE, FRAME_ENTRY_SIZE,
    MAX_PALETTE_COLOURS, PALETTE_ENTRY_SIZE, RLE_RUN_FLAG,
};

/// Signature of every shipped entry (`AMOU`). The clients read but never check it.
pub const AMOU_SIGNATURE: [u8; 4] = *b"AMOU";
/// Header version of every shipped entry. The clients read but never check it.
pub const AMOU_VERSION: u32 = 1;
/// Size of the header; the palette starts right after it in every shipped entry.
pub const AMOU_HEADER_SIZE: usize = 40;

/// Coverage of a transparent pixel (not coded).
pub const COVERAGE_TRANSPARENT: u8 = 0;
/// Coverage of a solid pixel: an edge factor of `EDGE_FACTOR_ONE` sixteenths.
pub const COVERAGE_SOLID: u8 = EDGE_FACTOR_ONE as u8;

/// Most pixels one skip byte covers, and most solid pixels in one coded run (`RLE_RUN_FLAG - 1`).
const MAX_RUN_LENGTH: usize = RLE_RUN_FLAG as usize - 1;
/// Header field offsets.
const VERSION_OFFSET: usize = 4;
const TOTAL_SIZE_OFFSET: usize = 8;
const ANIMATION_ID_OFFSET: usize = 12;
const BOUNDS_OFFSET: usize = 16;
const COLOURS_COUNT_OFFSET: usize = 24;
const COLOURS_OFFSET_OFFSET: usize = 28;
const FRAMES_COUNT_OFFSET: usize = 32;
const FRAMES_OFFSET_OFFSET: usize = 36;
const U16_SIZE: usize = 2;
const U32_SIZE: usize = 4;
/// Offset of the data offset field in a frame entry.
const ENTRY_DATA_OFFSET_FIELD: usize = 12;

/// A frame as palette indices plus per-pixel coverage, the form the RLE stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFrame {
    pub width: u16,
    pub height: u16,
    /// Anchor inside the frame image, `(-x0, -y0)` of the frame box (as [`super::DecodedFrame`]).
    pub center_x: i16,
    pub center_y: i16,
    /// Palette index of each pixel, row-major; 0 for transparent pixels.
    pub indices: Vec<u8>,
    /// Coverage of each pixel in sixteenths: [`COVERAGE_TRANSPARENT`], 1..15 (edge) or [`COVERAGE_SOLID`].
    pub coverage: Vec<u8>,
}

impl IndexedFrame {
    /// Frame box `(x0, y0, x1, y1)` relative to the anchor.
    pub fn frame_box(&self) -> eyre::Result<(i16, i16, i16, i16)> {
        let x0 = self.center_x.checked_neg();
        let y0 = self.center_y.checked_neg();
        let (Some(x0), Some(y0)) = (x0, y0) else {
            eyre::bail!("anchor ({}, {}) has no frame box", self.center_x, self.center_y);
        };
        let x1 = i16::try_from(i32::from(x0) + i32::from(self.width));
        let y1 = i16::try_from(i32::from(y0) + i32::from(self.height));
        match (x1, y1) {
            (Ok(x1), Ok(y1)) => Ok((x0, y0, x1, y1)),
            _ => eyre::bail!(
                "a {}x{} frame anchored at ({}, {}) does not fit the i16 frame box",
                self.width,
                self.height,
                self.center_x,
                self.center_y
            ),
        }
    }

    fn check_shape(&self) -> eyre::Result<usize> {
        let pixel_count = usize::from(self.width) * usize::from(self.height);
        eyre::ensure!(
            self.indices.len() == pixel_count && self.coverage.len() == pixel_count,
            "a {}x{} frame needs {pixel_count} indices and coverages, found {} and {}",
            self.width,
            self.height,
            self.indices.len(),
            self.coverage.len()
        );
        if let Some(bad) = self.coverage.iter().find(|&&coverage| coverage > COVERAGE_SOLID) {
            eyre::bail!("coverage {bad} is above {COVERAGE_SOLID}");
        }
        Ok(pixel_count)
    }
}

/// Encodes a frame as the shipped encoder does (every shipped EC and KR frame re-encodes byte for byte):
///
/// - transparent pixels are skip bytes of at most 127, carried over rows; the trailing transparent pixels are
///   skipped too, up to the end of the box;
/// - a coded run starts at the first coded pixel and never passes the end of its row (the client writes a run as a
///   linear span); it takes, in order, one leading edge pixel (when the first pixel is an edge and is not the last
///   of its row), up to 127 solid pixels, and one trailing edge pixel (when the solid count is below 127);
/// - an edge pixel that cannot lead or trail starts its own run (`0x80`, then a single trailing edge).
///
/// Fails when an index is not below `colour_count`.
pub fn encode_frame_rle(frame: &IndexedFrame, colour_count: usize) -> eyre::Result<Vec<u8>> {
    let pixel_count = frame.check_shape()?;
    let width = usize::from(frame.width);
    if let Some(index) = frame
        .indices
        .iter()
        .zip(&frame.coverage)
        .find(|&(&index, &coverage)| coverage != COVERAGE_TRANSPARENT && usize::from(index) >= colour_count)
        .map(|(&index, _)| index)
    {
        eyre::bail!("palette index {index} is not below the {colour_count} colours");
    }

    let is_edge = |i: usize| frame.coverage[i] != COVERAGE_TRANSPARENT && frame.coverage[i] != COVERAGE_SOLID;
    let mut out = Vec::new();
    let mut skip = 0usize;
    let mut i = 0usize;
    while i < pixel_count {
        if frame.coverage[i] == COVERAGE_TRANSPARENT {
            skip += 1;
            i += 1;
            continue;
        }
        push_skips(&mut out, &mut skip);
        let row_end = (i / width + 1) * width;

        let mut lead = None;
        if is_edge(i) && i + 1 < row_end {
            lead = Some(i);
            i += 1;
        }
        let solid_start = i;
        while i < row_end && frame.coverage[i] == COVERAGE_SOLID && i - solid_start < MAX_RUN_LENGTH {
            i += 1;
        }
        let solid_end = i;
        let mut trail = None;
        if solid_end - solid_start < MAX_RUN_LENGTH && i < row_end && is_edge(i) {
            trail = Some(i);
            i += 1;
        }

        let factor = |pixel: Option<usize>| pixel.map_or(0, |pixel| frame.coverage[pixel]);
        out.push(RLE_RUN_FLAG + (solid_end - solid_start) as u8);
        out.push((factor(lead) << EDGE_FACTOR_BITS) | (factor(trail) & EDGE_FACTOR_MASK));
        out.extend(lead.map(|pixel| frame.indices[pixel]));
        out.extend_from_slice(&frame.indices[solid_start..solid_end]);
        out.extend(trail.map(|pixel| frame.indices[pixel]));
    }
    push_skips(&mut out, &mut skip);
    Ok(out)
}

fn push_skips(out: &mut Vec<u8>, skip: &mut usize) {
    while *skip > 0 {
        let step = (*skip).min(MAX_RUN_LENGTH);
        out.push(step as u8);
        *skip -= step;
    }
}

/// One frame entry of an AMOU file with its pixel data, as stored (EC/KR: the RLE stream; Classic
/// `animationlegacyframe`: palette, size and Classic RLE, kept opaque).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmouFrameRecord {
    pub action: u16,
    pub frame_number: u16,
    pub x0: i16,
    pub y0: i16,
    pub x1: i16,
    pub y1: i16,
    pub data: Vec<u8>,
}

impl AmouFrameRecord {
    fn key(&self) -> (u16, u16) {
        (self.action, self.frame_number)
    }
}

/// An AMOU entry (`build/animationframe/%06d/%02d.bin`, also the Classic `animationlegacyframe` entries) as
/// header fields, palette and frame records, written back by [`Self::to_bytes`] in the shipped layout: the 40-byte
/// header, the palette, the frame table sorted by (action, frame number), then the frame data in table order with
/// no padding; each data offset is relative to its own table entry; `total_size` is the entry size before
/// [`Self::tail`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmouFile {
    pub version: u32,
    /// Header +0x0c, the body of the entry name; the clients never read it.
    pub animation_id: u32,
    /// Header box; the clients never read it. In EC and KR entries it is [`Self::frame_union_bounds`].
    pub bounds: AnimationBounds,
    /// Palette entries `[b0, b1, b2, keep_colour]`.
    pub colours: Vec<[u8; PALETTE_ENTRY_SIZE]>,
    pub frames: Vec<AmouFrameRecord>,
    /// Bytes after `total_size` (zero padding in some Classic entries); empty in EC and KR entries.
    pub tail: Vec<u8>,
}

/// Why [`AmouFile::parse`] could not keep an entry's exact layout; [`AmouFile::to_bytes`] then writes the shipped
/// layout, which differs from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AmouLayout {
    /// Frame data in table order, each stream ending where the next starts, the first right after the table.
    pub contiguous: bool,
    /// Palette right after the header and the frame table right after the palette.
    pub packed_tables: bool,
    /// Frame table strictly ascending by (action, frame number).
    pub sorted: bool,
}

impl AmouLayout {
    /// Whether [`AmouFile::to_bytes`] reproduces the parsed bytes.
    pub fn is_canonical(&self) -> bool {
        self.contiguous && self.packed_tables && self.sorted
    }
}

impl AmouFile {
    /// Splits an entry into its records. Each frame's data runs from its offset to the next larger offset of any
    /// entry (or to `total_size`). Returns the layout facts that decide whether [`Self::to_bytes`] is the identity.
    pub fn parse(data: &[u8]) -> eyre::Result<(Self, AmouLayout)> {
        let metadata = AnimationFrame::load_metadata(data)?;
        let read_u32 = |offset: usize| u32::from_le_bytes(data[offset..offset + U32_SIZE].try_into().unwrap());
        let read_i16 = |offset: usize| i16::from_le_bytes(data[offset..offset + U16_SIZE].try_into().unwrap());
        let colours_offset = read_u32(COLOURS_OFFSET_OFFSET) as usize;
        let colours_count = read_u32(COLOURS_COUNT_OFFSET) as usize;
        let frames_offset = metadata.frames_offset as usize;
        let table_end = frames_offset + metadata.frames.len() * FRAME_ENTRY_SIZE;
        let total_size = metadata.total_size as usize;
        eyre::ensure!(
            total_size >= table_end && total_size <= data.len(),
            "total size {total_size} is outside {table_end}..={}",
            data.len()
        );

        let starts: Vec<usize> = metadata.frames.iter().map(|frame| frame.data_offset as usize).collect();
        for (index, &start) in starts.iter().enumerate() {
            eyre::ensure!(
                (table_end..=total_size).contains(&start),
                "frame {index} data offset {start} is outside {table_end}..={total_size}"
            );
        }
        // In table order when the offsets ascend (so an empty stream stays empty); otherwise each stream ends at
        // the next larger offset.
        let ascending = starts.windows(2).all(|pair| pair[0] <= pair[1]);
        let mut sorted_starts = starts.clone();
        sorted_starts.sort_unstable();
        sorted_starts.dedup();
        let end_of = |index: usize, start: usize| {
            if ascending {
                return starts.get(index + 1).copied().unwrap_or(total_size);
            }
            let next = sorted_starts.partition_point(|&other| other <= start);
            sorted_starts.get(next).copied().unwrap_or(total_size)
        };

        let contiguous = ascending && starts.first().is_none_or(|&first| first == table_end);
        let mut frames = Vec::with_capacity(metadata.frames.len());
        for (index, entry) in metadata.frames.iter().enumerate() {
            let start = starts[index];
            let end = end_of(index, start);
            frames.push(record_of(entry, data[start..end].to_vec()));
        }

        let sorted = frames.windows(2).all(|pair| pair[0].key() < pair[1].key());
        let packed_tables =
            colours_offset == AMOU_HEADER_SIZE && frames_offset == colours_offset + colours_count * PALETTE_ENTRY_SIZE;
        let file = Self {
            version: read_u32(VERSION_OFFSET),
            animation_id: read_u32(ANIMATION_ID_OFFSET),
            bounds: AnimationBounds {
                x0: read_i16(BOUNDS_OFFSET),
                y0: read_i16(BOUNDS_OFFSET + U16_SIZE),
                x1: read_i16(BOUNDS_OFFSET + 2 * U16_SIZE),
                y1: read_i16(BOUNDS_OFFSET + 3 * U16_SIZE),
            },
            colours: metadata.colours,
            frames,
            tail: data[total_size..].to_vec(),
        };
        let signature_ok = data[..AMOU_SIGNATURE.len()] == AMOU_SIGNATURE;
        let layout = AmouLayout { contiguous, packed_tables: packed_tables && signature_ok, sorted };
        Ok((file, layout))
    }

    /// Builds an EC/KR entry from indexed frames keyed by (action, frame number): RLE per [`encode_frame_rle`],
    /// version [`AMOU_VERSION`], header box [`Self::frame_union_bounds`]. Fails on a repeated key, more than 256
    /// colours or an index past the palette.
    pub fn from_indexed_frames(
        animation_id: u32,
        colours: Vec<[u8; PALETTE_ENTRY_SIZE]>,
        frames: &[(u16, u16, IndexedFrame)],
    ) -> eyre::Result<Self> {
        eyre::ensure!(
            colours.len() <= MAX_PALETTE_COLOURS,
            "{} colours: a frame index is one byte, so at most {MAX_PALETTE_COLOURS} are usable",
            colours.len()
        );
        let mut records = Vec::with_capacity(frames.len());
        for (action, frame_number, frame) in frames {
            let (x0, y0, x1, y1) =
                frame.frame_box().wrap_err_with(|| format!("action {action} frame {frame_number}"))?;
            let data = encode_frame_rle(frame, colours.len())
                .wrap_err_with(|| format!("action {action} frame {frame_number}"))?;
            records.push(AmouFrameRecord { action: *action, frame_number: *frame_number, x0, y0, x1, y1, data });
        }
        records.sort_by_key(AmouFrameRecord::key);
        if let Some(pair) = records.windows(2).find(|pair| pair[0].key() == pair[1].key()) {
            eyre::bail!("action {} frame {} repeats", pair[0].action, pair[0].frame_number);
        }
        let mut file = Self {
            version: AMOU_VERSION,
            animation_id,
            bounds: AnimationBounds::default(),
            colours,
            frames: records,
            tail: Vec::new(),
        };
        file.bounds = file.frame_union_bounds();
        Ok(file)
    }

    /// The header box of EC and KR entries: the union of every frame box and of its mirror (`x -> -x`), so
    /// `x1 == -x0`; all zero without frames.
    pub fn frame_union_bounds(&self) -> AnimationBounds {
        let mut frames = self.frames.iter();
        let Some(first) = frames.next() else {
            return AnimationBounds::default();
        };
        let mirrored_x0 = |frame: &AmouFrameRecord| frame.x0.min(frame.x1.saturating_neg());
        let mirrored_x1 = |frame: &AmouFrameRecord| frame.x1.max(frame.x0.saturating_neg());
        let mut bounds =
            AnimationBounds { x0: mirrored_x0(first), y0: first.y0, x1: mirrored_x1(first), y1: first.y1 };
        for frame in frames {
            bounds.x0 = bounds.x0.min(mirrored_x0(frame));
            bounds.y0 = bounds.y0.min(frame.y0);
            bounds.x1 = bounds.x1.max(mirrored_x1(frame));
            bounds.y1 = bounds.y1.max(frame.y1);
        }
        bounds
    }

    /// The entry bytes. Fails when the frame table is not strictly ascending by (action, frame number): the
    /// clients binary-search it (`UOAnimations::FindFrameEntry`), so an unsorted table loses frames.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        if let Some(pair) = self.frames.windows(2).find(|pair| pair[0].key() >= pair[1].key()) {
            eyre::bail!(
                "frame table not ascending: action {} frame {} before action {} frame {}",
                pair[0].action,
                pair[0].frame_number,
                pair[1].action,
                pair[1].frame_number
            );
        }
        let colours_offset = AMOU_HEADER_SIZE;
        let frames_offset = colours_offset + self.colours.len() * PALETTE_ENTRY_SIZE;
        let table_end = frames_offset + self.frames.len() * FRAME_ENTRY_SIZE;
        let total_size = table_end + self.frames.iter().map(|frame| frame.data.len()).sum::<usize>();
        let to_u32 = |value: usize, what: &str| {
            u32::try_from(value).map_err(|_| eyre::eyre!("{what} {value} does not fit the u32 header field"))
        };

        let mut out = Vec::with_capacity(total_size + self.tail.len());
        out.extend_from_slice(&AMOU_SIGNATURE);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&to_u32(total_size, "total size")?.to_le_bytes());
        out.extend_from_slice(&self.animation_id.to_le_bytes());
        for value in [self.bounds.x0, self.bounds.y0, self.bounds.x1, self.bounds.y1] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&to_u32(self.colours.len(), "colour count")?.to_le_bytes());
        out.extend_from_slice(&to_u32(colours_offset, "colours offset")?.to_le_bytes());
        out.extend_from_slice(&to_u32(self.frames.len(), "frame count")?.to_le_bytes());
        out.extend_from_slice(&to_u32(frames_offset, "frames offset")?.to_le_bytes());
        debug_assert_eq!(out.len(), AMOU_HEADER_SIZE);
        for colour in &self.colours {
            out.extend_from_slice(colour);
        }

        let mut data_start = table_end;
        for (index, frame) in self.frames.iter().enumerate() {
            let entry_start = frames_offset + index * FRAME_ENTRY_SIZE;
            out.extend_from_slice(&frame.action.to_le_bytes());
            out.extend_from_slice(&frame.frame_number.to_le_bytes());
            for value in [frame.x0, frame.y0, frame.x1, frame.y1] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            debug_assert_eq!(out.len(), entry_start + ENTRY_DATA_OFFSET_FIELD);
            out.extend_from_slice(&to_u32(data_start - entry_start, "frame data offset")?.to_le_bytes());
            data_start += frame.data.len();
        }
        for frame in &self.frames {
            out.extend_from_slice(&frame.data);
        }
        debug_assert_eq!(out.len(), total_size);
        out.extend_from_slice(&self.tail);
        Ok(out)
    }

    /// The (action, frame number) keys, in table order.
    pub fn frame_keys(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        self.frames.iter().map(AmouFrameRecord::key)
    }
}

fn record_of(entry: &FrameEntry, data: Vec<u8>) -> AmouFrameRecord {
    AmouFrameRecord {
        action: entry.action,
        frame_number: entry.frame_number,
        x0: entry.init_coords_x,
        y0: entry.init_coords_y,
        x1: entry.end_coords_x,
        y1: entry.end_coords_y,
        data,
    }
}

/// Coverage (sixteenths) whose decoded alpha `255 * f >> 4` (255 for solid, 0 for transparent) is nearest to
/// `alpha`; ties go to the lower coverage. Every decoded alpha maps back to its own coverage.
pub fn coverage_of_alpha(alpha: u8) -> u8 {
    (COVERAGE_TRANSPARENT..=COVERAGE_SOLID)
        .min_by_key(|&coverage| (i16::from(alpha_of_coverage(coverage)) - i16::from(alpha)).abs())
        .unwrap_or(COVERAGE_TRANSPARENT)
}

/// Straight alpha of a pixel with this coverage, as the decoder gives it.
pub fn alpha_of_coverage(coverage: u8) -> u8 {
    match coverage {
        COVERAGE_TRANSPARENT => 0,
        COVERAGE_SOLID.. => super::OPAQUE_ALPHA,
        edge => super::edge_alpha(edge),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u16, height: u16, pixels: &[(u8, u8)]) -> IndexedFrame {
        IndexedFrame {
            width,
            height,
            center_x: 0,
            center_y: 0,
            indices: pixels.iter().map(|pixel| pixel.0).collect(),
            coverage: pixels.iter().map(|pixel| pixel.1).collect(),
        }
    }

    const T: (u8, u8) = (0, COVERAGE_TRANSPARENT);

    #[test]
    fn runs_take_a_leading_edge_solids_and_a_trailing_edge() {
        let pixels = [(1, 8), (2, COVERAGE_SOLID), (3, 4), T];
        let bytes = encode_frame_rle(&frame(4, 1, &pixels), 4).unwrap();
        assert_eq!(bytes, [RLE_RUN_FLAG + 1, 0x84, 1, 2, 3, 1]);
    }

    #[test]
    fn an_edge_at_the_row_end_trails_an_empty_run_and_skips_carry_over_rows() {
        // Row 0: transparent, edge (last of its row). Row 1: transparent, transparent.
        let pixels = [T, (2, 5), T, T];
        let bytes = encode_frame_rle(&frame(2, 2, &pixels), 3).unwrap();
        assert_eq!(bytes, [1, RLE_RUN_FLAG, 0x05, 2, 2]);
    }

    #[test]
    fn runs_stop_at_the_row_end_and_at_127_solids() {
        let pixels = [(1, COVERAGE_SOLID); 2];
        assert_eq!(
            encode_frame_rle(&frame(1, 2, &pixels), 2).unwrap(),
            [RLE_RUN_FLAG + 1, 0, 1, RLE_RUN_FLAG + 1, 0, 1]
        );
        let mut long = vec![(1, COVERAGE_SOLID); MAX_RUN_LENGTH + 1];
        long.push((1, 3));
        let bytes = encode_frame_rle(&frame(long.len() as u16, 1, &long), 2).unwrap();
        // 127 solids without a trailing edge, then one solid with the edge.
        assert_eq!(bytes[0], RLE_RUN_FLAG + MAX_RUN_LENGTH as u8);
        assert_eq!(bytes[1], 0);
        assert_eq!(&bytes[2 + MAX_RUN_LENGTH..], &[RLE_RUN_FLAG + 1, 0x03, 1, 1]);
    }

    #[test]
    fn encoding_rejects_indices_past_the_palette() {
        assert!(encode_frame_rle(&frame(1, 1, &[(2, COVERAGE_SOLID)]), 2).is_err());
        // Transparent pixels carry no index.
        assert!(encode_frame_rle(&frame(1, 1, &[(9, COVERAGE_TRANSPARENT)]), 2).is_ok());
    }

    #[test]
    fn every_decoded_alpha_maps_back_to_its_coverage() {
        for coverage in COVERAGE_TRANSPARENT..=COVERAGE_SOLID {
            assert_eq!(coverage_of_alpha(alpha_of_coverage(coverage)), coverage);
        }
        assert_eq!(coverage_of_alpha(7), COVERAGE_TRANSPARENT);
        assert_eq!(coverage_of_alpha(250), COVERAGE_SOLID);
    }

    #[test]
    fn container_round_trips_and_decodes() {
        let colours = vec![[0, 0, 0, 0], [200, 10, 10, 1], [90, 90, 90, 0]];
        let mut first = frame(3, 2, &[(1, 8), (2, COVERAGE_SOLID), (1, 4), T, (2, COVERAGE_SOLID), T]);
        first.center_x = 1;
        first.center_y = 2;
        let second = frame(1, 1, &[(2, COVERAGE_SOLID)]);
        let file =
            AmouFile::from_indexed_frames(7, colours.clone(), &[(0, 2, second.clone()), (0, 1, first.clone())]).unwrap();
        assert_eq!(file.frame_keys().collect::<Vec<_>>(), [(0, 1), (0, 2)]);
        // Union of [-1, 2] and its mirror [-2, 1]; y from -2..0 and 0..1.
        assert_eq!(file.bounds, AnimationBounds { x0: -2, y0: -2, x1: 2, y1: 1 });

        let bytes = file.to_bytes().unwrap();
        let (parsed, layout) = AmouFile::parse(&bytes).unwrap();
        assert!(layout.is_canonical());
        assert_eq!(parsed, file);

        let animation = AnimationFrame::load(&bytes).unwrap();
        assert_eq!(animation.decode_frame_indexed(&animation.frames[0]).unwrap(), first);
        assert_eq!(animation.decode_frame_indexed(&animation.frames[1]).unwrap(), second);
        let decoded = animation.decode_frame(&animation.frames[0]).unwrap();
        assert_eq!(&decoded.data[..4], &[200, 10, 10, alpha_of_coverage(8)]);
    }

    #[test]
    fn parse_reports_non_canonical_layouts_and_keeps_the_tail() {
        let file = AmouFile::from_indexed_frames(1, vec![[1, 2, 3, 0]], &[(0, 1, frame(1, 1, &[(0, COVERAGE_SOLID)]))])
            .unwrap();
        let mut bytes = file.to_bytes().unwrap();
        bytes.extend_from_slice(&[0, 0, 0]);
        let (parsed, layout) = AmouFile::parse(&bytes).unwrap();
        assert!(layout.is_canonical());
        assert_eq!(parsed.tail, [0, 0, 0]);
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn repeated_keys_and_unsorted_tables_are_refused() {
        let one = frame(1, 1, &[(0, COVERAGE_SOLID)]);
        assert!(AmouFile::from_indexed_frames(1, vec![[0; 4]], &[(0, 1, one.clone()), (0, 1, one.clone())]).is_err());
        let mut file = AmouFile::from_indexed_frames(1, vec![[0; 4]], &[(0, 1, one.clone()), (0, 2, one)]).unwrap();
        file.frames.swap(0, 1);
        assert!(file.to_bytes().is_err());
    }
}
